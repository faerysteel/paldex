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
    let level_path = world.path.join("Level.sav");
    let raw = std::fs::read(&level_path)?;
    let level_mtime = std::fs::metadata(&level_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok());

    let (gvas, _) = paldex_sav::decompress(&raw)?;
    let level_hash = sha256_hex(&gvas);
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
    Ok(store.ingest_snapshot(&world.id, &input)?)
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
    }
}
