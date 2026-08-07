//! Orchestrates a full decode → ingest pass for one world: `Level.sav` for
//! Pals/guilds/base camps, each `Players/<uid>.sav` for progression, then
//! [`paldex_store::Store::ingest_snapshot`]. This is app-layer glue, not
//! general-purpose library material — it's the one place that needs
//! `paldex-sav`, `paldex-gvas`, `paldex-model`, and `paldex-store` all at
//! once, cross-cutting every crate below it.

use std::path::Path;

use paldex_gvas::{StructValue, Value};
use paldex_locate::World;
use paldex_store::{SnapshotInput, Store};

#[derive(Debug)]
pub struct SyncError(pub String);

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

macro_rules! impl_from_display {
    ($($ty:ty),+ $(,)?) => {
        $(impl From<$ty> for SyncError {
            fn from(e: $ty) -> Self {
                SyncError(e.to_string())
            }
        })+
    };
}
impl_from_display!(
    std::io::Error,
    paldex_sav::SavError,
    paldex_gvas::GvasError,
    paldex_store::StoreError
);

/// Decode `world` fresh from disk and ingest it into `store`, returning the
/// new snapshot's id.
pub fn sync_world(store: &mut Store, world: &World) -> Result<i64, SyncError> {
    sync_world_inner(store, world, true).map(|outcome| match outcome {
        SyncOutcome::Ingested(id) => id,
        // `force = false` is the only way to get `Unchanged`, and this call
        // passes `true`.
        SyncOutcome::Unchanged => unreachable!("forced sync always ingests"),
    })
}

/// What a sync did — distinguished because the watcher fires on every write
/// Palworld makes, and Palworld rewrites `Level.sav` whether or not anything
/// the tracker cares about changed. Ingesting regardless would grow a snapshot
/// per autosave and push real history out of the retention window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    Ingested(i64),
    /// The decompressed `Level.sav` hashed identically to the latest stored
    /// snapshot, so nothing was written.
    Unchanged,
}

/// [`sync_world`], but skips ingest when the world's content hash already
/// matches the newest stored snapshot.
///
/// The save still has to be read, decompressed, and hashed to know that — the
/// short-circuit saves the GVAS parse, the RawData decode, and the write, not
/// the I/O.
pub fn sync_world_if_changed(store: &mut Store, world: &World) -> Result<SyncOutcome, SyncError> {
    sync_world_inner(store, world, false)
}

fn sync_world_inner(store: &mut Store, world: &World, force: bool) -> Result<SyncOutcome, SyncError> {
    let level_path = world.path.join("Level.sav");
    let raw = std::fs::read(&level_path)?;
    let level_mtime = std::fs::metadata(&level_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok());

    let (gvas, _) = paldex_sav::decompress(&raw)?;
    let level_hash = sha256_hex(&gvas);
    if !force && store.latest_hash_matches(&world.id, &level_hash)? {
        return Ok(SyncOutcome::Unchanged);
    }
    let parsed = paldex_gvas::parse(&gvas)?;

    let Some(Value::Struct {
        value: StructValue::Properties(world_data),
        ..
    }) = parsed.get("worldSaveData")
    else {
        return Err(SyncError("worldSaveData missing or malformed".to_owned()));
    };

    let map_entries = |field: &str| -> Vec<(Value, Value)> {
        match world_data.iter().find(|p| p.name == field).map(|p| &p.value) {
            Some(Value::Map(entries)) => entries.clone(),
            _ => Vec::new(),
        }
    };

    let character_result = paldex_model::decode_character_map(&map_entries("CharacterSaveParameterMap"));
    let guilds = paldex_model::decode_group_map(&map_entries("GroupSaveDataMap"));
    let guild_ids: Vec<_> = guilds
        .iter()
        .filter(|g| g.kind == paldex_model::GroupKind::Guild)
        .map(|g| g.id)
        .collect();
    let base_camps = paldex_model::decode_base_camp_map(&map_entries("BaseCampSaveData"), &guild_ids);

    let mut pals = character_result.pals;
    let mut players = Vec::new();
    for player_uid in &world.players {
        let path = world.path.join("Players").join(format!("{}.sav", player_uid.0));
        let Ok(raw) = std::fs::read(&path) else { continue };
        let Ok((gvas, _)) = paldex_sav::decompress(&raw) else { continue };
        let Ok(player_root) = paldex_gvas::parse(&gvas) else { continue };
        if let Ok(player) = paldex_model::decode_player(&player_root) {
            paldex_model::resolve_locations(&mut pals, &player);
            players.push(player);
        }
    }

    let taken_at = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| SyncError("system clock is before the Unix epoch".to_owned()))?
            .as_millis(),
    )
    .map_err(|_| SyncError("system clock too far in the future".to_owned()))?;

    let input = SnapshotInput {
        taken_at,
        level_mtime,
        level_hash,
        pals,
        players,
        guilds,
        base_camps,
    };

    store
        .upsert_world(&world.id, "local", &world.id, &world.path.display().to_string())?;
    Ok(SyncOutcome::Ingested(store.ingest_snapshot(&world.id, &input)?))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Re-resolve a `World` by its own directory path — the frontend round-trips
/// the `path` a prior `list_saves`/`resolve_folder` call already gave it,
/// rather than the whole `World` struct, so commands that act on a specific
/// world need to look it back up.
pub fn find_world_by_path(world_path: &Path) -> Option<World> {
    let parent = world_path.parent()?; // the per-Steam-ID directory
    paldex_locate::resolve_manual(parent)
        .ok()?
        .into_iter()
        .find_map(|root| root.worlds().into_iter().find(|w| w.path == world_path))
}

#[cfg(test)]
mod tests {
    //! Exercises the exact code path the Tauri commands use — `sync_world`
    //! then the `queries` module — against a real save. This is the
    //! integration test for the app-layer glue itself; every crate it calls
    //! into already has its own real-save tests, but nothing else exercised
    //! `sync_world`'s orchestration or `queries::*` before this.
    use super::*;
    use paldex_locate::discover;

    fn real_trackable_world() -> Option<World> {
        if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
            paldex_locate::resolve_manual(Path::new(&dir))
                .ok()?
                .into_iter()
                .find_map(|r| r.worlds().into_iter().find(World::is_trackable))
        } else {
            discover()
                .into_iter()
                .find_map(|r| r.worlds().into_iter().find(World::is_trackable))
        }
    }

    /// The watcher fires on every write Palworld makes, and Palworld rewrites
    /// `Level.sav` on a timer whether or not anything changed. Without this
    /// short-circuit every autosave would append a snapshot and push real
    /// history out of the retention window.
    #[test]
    fn an_unchanged_world_is_not_re_ingested() {
        let Some(world) = real_trackable_world() else {
            eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
            return;
        };

        let mut store = Store::open_in_memory().expect("open in-memory store");
        let first = sync_world_if_changed(&mut store, &world).expect("first sync");
        assert!(
            matches!(first, SyncOutcome::Ingested(_)),
            "an empty store must ingest, got {first:?}"
        );

        let second = sync_world_if_changed(&mut store, &world).expect("second sync");
        assert_eq!(
            second,
            SyncOutcome::Unchanged,
            "re-syncing an untouched world must not create a second snapshot"
        );

        let count: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
            .expect("count snapshots");
        assert_eq!(count, 1, "expected exactly one snapshot, got {count}");

        // `sync_world` is the forced path behind Resync — it must still ingest
        // even when nothing changed, or the button would look broken.
        sync_world(&mut store, &world).expect("forced resync");
        let forced: i64 = store
            .conn()
            .query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0))
            .expect("count snapshots");
        assert_eq!(forced, 2, "a forced resync must ingest regardless of the hash");
    }

    #[test]
    fn sync_world_then_queries_round_trip_against_a_real_save() {
        let Some(world) = real_trackable_world() else {
            eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
            return;
        };

        let mut store = Store::open_in_memory().expect("open in-memory store");
        let snapshot_id = sync_world(&mut store, &world).expect("sync_world");
        assert!(snapshot_id > 0);

        let summary = crate::queries::snapshot_summary(&store, snapshot_id).expect("snapshot_summary");
        eprintln!(
            "synced {} pals, {} players",
            summary.pal_count, summary.player_count
        );
        assert!(summary.pal_count > 0, "expected at least one pal in a real world");

        let roster = crate::queries::pal_roster(&store, snapshot_id).expect("pal_roster");
        assert_eq!(roster.len() as i64, summary.pal_count);
        assert!(
            roster.iter().all(|p| p.iv_hp <= 100 && p.iv_shot <= 100 && p.iv_defense <= 100),
            "every roster IV should be in 0..=100"
        );

        let dex = crate::queries::dex_progress(&store, &world.id).expect("dex_progress");
        eprintln!("dex: {} species unlocked", dex.unlocked_species_count);

        let players = crate::queries::player_progress(&store, snapshot_id).expect("player_progress");
        assert_eq!(players.len() as i64, summary.player_count);

        let bases = crate::queries::base_summary(&store, snapshot_id).expect("base_summary");
        eprintln!("{} base camps", bases.len());

        let flags = crate::queries::player_flags_detail(&store, snapshot_id).expect("player_flags_detail");
        assert_eq!(flags.len() as i64, summary.player_count);
        for f in &flags {
            eprintln!(
                "  player {}: {} tech, {} normal bosses, {} quests",
                &f.player_uid[..8.min(f.player_uid.len())],
                f.unlocked_tech.len(),
                f.normal_boss_defeated.len(),
                f.completed_quests.len(),
            );
        }
    }
}
