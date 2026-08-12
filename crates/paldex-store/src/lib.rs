//! Local SQLite store: current-state tables replaced wholesale per snapshot,
//! plus the append-only `dex_events` table (dex completion must never shrink
//! — see `migrations/0001_init.sql`).

mod ingest;
mod watch;

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub use ingest::SnapshotInput;
pub use watch::{read_when_stable, watch, watch_until, WatchConfig, WatchError};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("system clock is before the Unix epoch")]
    ClockError,
}

/// Schema version stamped into `PRAGMA user_version`. Bump when adding a
/// migration, and add it to [`Store::migrate`]'s ladder.
const SCHEMA_VERSION: i32 = 2;

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if absent) a database file and apply migrations.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// An in-memory database — used by tests, and available for anything
    /// that wants a scratch store without touching disk.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::from_connection(conn)
    }

    fn from_connection(conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrate(&conn)?;
        Ok(Self { conn })
    }

    /// Bring a database up to [`SCHEMA_VERSION`], skipping work already done.
    ///
    /// Tracked via `PRAGMA user_version` rather than `IF NOT EXISTS` on each
    /// statement: `IF NOT EXISTS` would let a *future* schema change silently
    /// no-op against an old database, which is a far worse failure than an
    /// error. The version stamp makes "which migrations has this file seen"
    /// explicit.
    fn migrate(conn: &Connection) -> Result<(), StoreError> {
        let mut version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }

        // Databases created before versioning existed carry a schema but a
        // version of 0. Re-running DDL against them fails on `table worlds
        // already exists`, so work out what they actually have and enter the
        // ladder there. Inferred from the schema rather than assumed to be
        // version 1: a database can reach `user_version = 0` while holding a
        // *later* schema, and stamping it straight to SCHEMA_VERSION would
        // instead skip migrations it genuinely needs.
        if version == 0 {
            version = Self::detected_version(conn)?;
        }

        // One step per version, in order, each guarded by the version it
        // upgrades *from*. A database at any earlier version walks the whole
        // ladder; one at the current version does nothing.
        if version < 1 {
            conn.execute_batch(include_str!("../migrations/0001_init.sql"))?;
        }
        if version < 2 {
            conn.execute_batch(include_str!("../migrations/0002_player_identity.sql"))?;
        }

        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    /// Which schema an unstamped database already has, by looking for the
    /// marker each migration introduced. Returns 0 for an empty file, meaning
    /// the ladder runs from the beginning.
    fn detected_version(conn: &Connection) -> Result<i32, StoreError> {
        if !Self::has_table(conn, "worlds")? {
            return Ok(0);
        }
        if Self::has_column(conn, "players", "name")? {
            return Ok(2);
        }
        Ok(1)
    }

    fn has_table(conn: &Connection, table: &str) -> Result<bool, StoreError> {
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, StoreError> {
        // `pragma_table_info` takes the table name as an identifier, so it
        // can't be bound as a parameter; every caller passes a literal.
        let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Record (or refresh) a known save world.
    pub fn upsert_world(
        &self,
        id: &str,
        source: &str,
        steam_id: &str,
        path: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO worlds (id, source, steam_id, path, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET source = excluded.source,
                 steam_id = excluded.steam_id, path = excluded.path,
                 last_seen = excluded.last_seen",
            rusqlite::params![id, source, steam_id, path, now_millis()?],
        )?;
        Ok(())
    }

    /// Ingest one parsed snapshot: current-state tables are inserted fresh
    /// under a new `snapshot_id`; `dex_events` gets any newly-unlocked
    /// species appended (existing rows are never touched — see module docs).
    /// Returns the new snapshot's id.
    pub fn ingest_snapshot(
        &mut self,
        world_id: &str,
        input: &SnapshotInput,
    ) -> Result<i64, StoreError> {
        let tx = self.conn.transaction()?;
        let snapshot_id = ingest::insert_snapshot(&tx, world_id, input)?;
        ingest::insert_pals(&tx, snapshot_id, &input.pals)?;
        ingest::insert_players(&tx, snapshot_id, &input.players, &input.player_identities)?;
        ingest::insert_guilds(&tx, snapshot_id, &input.guilds)?;
        ingest::insert_base_camps(&tx, snapshot_id, &input.base_camps)?;
        ingest::append_dex_events(&tx, world_id, input.taken_at, &input.players)?;
        tx.commit()?;
        Ok(snapshot_id)
    }

    /// Keep only the `keep` most recent snapshots for a world (current-state
    /// child rows cascade via `ON DELETE CASCADE`; `dex_events` is untouched).
    pub fn prune_snapshots(&self, world_id: &str, keep: usize) -> Result<usize, StoreError> {
        let deleted = self.conn.execute(
            "DELETE FROM snapshots WHERE world_id = ?1 AND id NOT IN (
                 SELECT id FROM snapshots WHERE world_id = ?1
                 ORDER BY taken_at DESC LIMIT ?2
             )",
            rusqlite::params![world_id, keep as i64],
        )?;
        Ok(deleted)
    }

    /// Whether re-ingesting `level_hash` for `world_id` would be a no-op —
    /// the caller's short-circuit for "file changed but content didn't"
    /// (Palworld's non-atomic writes can produce a byte-identical rewrite).
    pub fn latest_hash_matches(&self, world_id: &str, level_hash: &str) -> Result<bool, StoreError> {
        let latest: Option<String> = self
            .conn
            .query_row(
                "SELECT level_hash FROM snapshots WHERE world_id = ?1
                 ORDER BY taken_at DESC LIMIT 1",
                [world_id],
                |row| row.get(0),
            )
            .ok();
        Ok(latest.as_deref() == Some(level_hash))
    }
}

impl Store {
    /// Direct connection access for queries beyond what the high-level API
    /// covers — read-only reporting queries (roster tables, dex progress),
    /// or test assertions on table contents.
    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// The most recent snapshot id for a world, if it has any.
    pub fn latest_snapshot_id(&self, world_id: &str) -> Result<Option<i64>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM snapshots WHERE world_id = ?1 ORDER BY taken_at DESC LIMIT 1",
                [world_id],
                |row| row.get(0),
            )
            .ok())
    }
}

pub(crate) fn now_millis() -> Result<i64, StoreError> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StoreError::ClockError)?
        .as_millis();
    i64::try_from(ms).map_err(|_| StoreError::ClockError)
}
