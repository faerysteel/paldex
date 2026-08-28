//! Reopening a database file must be a no-op, not an error.
//!
//! Regression test for a bug the rest of the suite could not see: every other
//! test uses `Store::open_in_memory()`, which is always a fresh database, so
//! re-running the schema DDL never collided. Against a real file it failed
//! with `table worlds already exists` on the second open — meaning the app
//! worked once and then broke on every subsequent launch.

use paldex_store::Store;

#[test]
fn reopening_an_existing_database_file_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("paldex.sqlite3");

    let first = Store::open(&path).expect("first open");
    first
        .upsert_world("world-1", "test", "steam-1", "/tmp/world-1")
        .expect("write through the first handle");
    drop(first);

    let second = Store::open(&path).expect("reopening an existing database should succeed");
    // Data written before the reopen must still be there — a migration that
    // silently recreated the schema would look like success but lose it.
    let worlds: i64 = second
        .conn()
        .query_row("SELECT COUNT(*) FROM worlds", [], |row| row.get(0))
        .expect("count worlds");
    assert_eq!(worlds, 1, "reopening must preserve existing rows");

    // A third open exercises the already-stamped path.
    Store::open(&path).expect("third open");
}

/// A database sitting at v2 walks the ladder to v3 without losing rows.
///
/// The v2 database is built by running the historical migrations rather than
/// by mutating a current one, so this exercises the same bytes a real user's
/// file has.
#[test]
fn migrates_a_v2_database_to_v3_and_keeps_its_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("v2.sqlite3");

    {
        let conn = rusqlite::Connection::open(&path).expect("create v2");
        conn.execute_batch(include_str!("../migrations/0001_init.sql")).expect("0001");
        conn.execute_batch(include_str!("../migrations/0002_player_identity.sql")).expect("0002");
        conn.pragma_update(None, "user_version", 2).expect("stamp v2");

        conn.execute(
            "INSERT INTO worlds (id, source, steam_id, path, last_seen)
             VALUES ('w', 'test', '0', '/tmp/w', 1)",
            [],
        )
        .expect("world");
        conn.execute(
            "INSERT INTO snapshots (world_id, taken_at, level_mtime, level_hash)
             VALUES ('w', 1, NULL, 'h')",
            [],
        )
        .expect("snapshot");
        let snapshot_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO base_camps (snapshot_id, id, guild_id) VALUES (?1, 'base-1', 'guild-1')",
            [snapshot_id],
        )
        .expect("base camp");
    }

    let store = Store::open(&path).expect("a v2 database should migrate to v3");

    let version: i32 = store
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("read version");
    assert_eq!(version, 3, "the ladder should have stamped v3");

    // The pre-existing row survives, with the new column NULL — "not decoded",
    // which is exactly right until the next sync re-ingests this world.
    let (id, guild_id, worker): (String, Option<String>, Option<String>) = store
        .conn()
        .query_row(
            "SELECT id, guild_id, worker_container_id FROM base_camps",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("the v2 base camp row should survive");
    assert_eq!(id, "base-1");
    assert_eq!(guild_id.as_deref(), Some("guild-1"));
    assert_eq!(worker, None);

    // Reopening is still a no-op now that it is stamped.
    Store::open(&path).expect("reopen at v3");
}

/// A database created fresh lands on the current version directly, without
/// walking the ladder.
#[test]
fn a_fresh_database_lands_on_the_current_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("fresh.sqlite3");

    let store = Store::open(&path).expect("create");
    let version: i32 = store
        .conn()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("read version");
    assert_eq!(version, 3);
}

/// Databases created before schema versioning have the tables but
/// `user_version = 0`; they must be adopted rather than re-migrated.
#[test]
fn adopts_an_unversioned_database_that_already_has_the_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("legacy.sqlite3");

    {
        let store = Store::open(&path).expect("create");
        store
            .conn()
            .pragma_update(None, "user_version", 0)
            .expect("simulate a pre-versioning database");
    }

    Store::open(&path).expect("an unversioned but fully-formed database should be adopted");
}
