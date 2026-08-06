//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way the other crates' real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If
//! neither yields a trackable (hosted) world, every test here skips rather than
//! failing.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use paldex_gvas::{StructValue, Value};
use paldex_locate::{discover, resolve_manual, World};
use paldex_store::{SnapshotInput, Store};

fn real_snapshot() -> Option<(World, SnapshotInput)> {
    let root = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        resolve_manual(std::path::Path::new(&dir))
            .unwrap_or_else(|e| panic!("PALDEX_TEST_SAVE_DIR is set but unusable: {e}"))
            .into_iter()
            .next()
    } else {
        discover().into_iter().next()
    }?;

    let world = root.worlds().into_iter().find(World::is_trackable)?;
    let raw = std::fs::read(world.path.join("Level.sav")).ok()?;
    let (gvas, _) = paldex_sav::decompress(&raw).ok()?;
    let parsed = paldex_gvas::parse(&gvas).ok()?;

    let Some(Value::Struct {
        value: StructValue::Properties(world_data),
        ..
    }) = parsed.get("worldSaveData")
    else {
        panic!("worldSaveData missing or not the expected generic-struct shape");
    };
    let map_entries = |field: &str| -> Vec<(Value, Value)> {
        match world_data.iter().find(|p| p.name == field).map(|p| &p.value) {
            Some(Value::Map(entries)) => entries.clone(),
            other => panic!("{field} missing or not a Map, got {other:?}"),
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

    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    let level_hash = format!("{:016x}", hasher.finish());

    let input = SnapshotInput {
        taken_at: 1,
        level_mtime: None,
        level_hash,
        pals,
        players,
        guilds,
        base_camps,
    };
    Some((world, input))
}

macro_rules! require_real_snapshot {
    () => {
        match real_snapshot() {
            Some(s) => s,
            None => {
                eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

#[test]
fn ingests_a_real_snapshot_and_counts_reconcile() {
    let (_world, input) = require_real_snapshot!();
    let pal_count = input.pals.len();
    let player_count = input.players.len();

    let mut store = Store::open_in_memory().unwrap();
    store.upsert_world("test-world", "test", "0", "/tmp/test").unwrap();
    let snapshot_id = store.ingest_snapshot("test-world", &input).unwrap();
    assert!(snapshot_id > 0);

    let conn_pal_count: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM pals WHERE snapshot_id = ?1", [snapshot_id], |r| r.get(0))
        .unwrap();
    assert_eq!(conn_pal_count as usize, pal_count);

    let conn_player_count: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM players WHERE snapshot_id = ?1", [snapshot_id], |r| r.get(0))
        .unwrap();
    assert_eq!(conn_player_count as usize, player_count);
}

#[test]
fn dex_events_are_monotonic_across_snapshots() {
    let (_world, input) = require_real_snapshot!();
    if input.players.is_empty() {
        eprintln!("skipping: no players decoded");
        return;
    }

    let mut store = Store::open_in_memory().unwrap();
    store.upsert_world("test-world", "test", "0", "/tmp/test").unwrap();

    // Snapshot A: full data.
    store.ingest_snapshot("test-world", &input).unwrap();
    let count_a: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM dex_events WHERE world_id = 'test-world'", [], |r| r.get(0))
        .unwrap();

    // Snapshot B: same players/dex data, but fewer pals in the world (as if
    // some were released) -- dex_events must not shrink.
    let mut input_b = SnapshotInput {
        taken_at: 2,
        level_mtime: input.level_mtime,
        level_hash: "different-hash".to_owned(),
        pals: input.pals.iter().take(input.pals.len() / 2).cloned().collect(),
        players: input.players.clone(),
        guilds: input.guilds.clone(),
        base_camps: input.base_camps.clone(),
    };
    // Also drop half of one player's paldeck_unlocked, simulating a parse
    // that (incorrectly) saw fewer unlocks -- dex_events must still hold the
    // originally-recorded entries.
    if let Some(p) = input_b.players.first_mut() {
        let half: std::collections::HashSet<String> =
            p.paldeck_unlocked.iter().take(p.paldeck_unlocked.len() / 2).cloned().collect();
        p.paldeck_unlocked = half;
    }
    store.ingest_snapshot("test-world", &input_b).unwrap();

    let count_b: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM dex_events WHERE world_id = 'test-world'", [], |r| r.get(0))
        .unwrap();

    eprintln!("dex_events: A={count_a}, B={count_b}");
    assert!(count_b >= count_a, "dex_events shrank from {count_a} to {count_b}");
}

#[test]
fn pruning_keeps_exactly_n_snapshots() {
    let (_world, input) = require_real_snapshot!();

    let mut store = Store::open_in_memory().unwrap();
    store.upsert_world("test-world", "test", "0", "/tmp/test").unwrap();

    for i in 0..5 {
        let s = SnapshotInput {
            taken_at: i,
            level_mtime: None,
            level_hash: format!("hash-{i}"),
            pals: input.pals.clone(),
            players: Vec::new(),
            guilds: Vec::new(),
            base_camps: Vec::new(),
        };
        store.ingest_snapshot("test-world", &s).unwrap();
    }

    let deleted = store.prune_snapshots("test-world", 2).unwrap();
    assert_eq!(deleted, 3);

    let remaining: i64 = store
        .conn()
        .query_row("SELECT COUNT(*) FROM snapshots WHERE world_id = 'test-world'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 2);
}

#[test]
fn reingesting_an_unchanged_hash_is_detected() {
    let (_world, input) = require_real_snapshot!();

    let mut store = Store::open_in_memory().unwrap();
    store.upsert_world("test-world", "test", "0", "/tmp/test").unwrap();
    store.ingest_snapshot("test-world", &input).unwrap();

    assert!(store.latest_hash_matches("test-world", &input.level_hash).unwrap());
    assert!(!store.latest_hash_matches("test-world", "some-other-hash").unwrap());
}
