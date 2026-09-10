//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way the other crates' real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If
//! neither yields a trackable (hosted) world, every test here skips rather than
//! failing, so CI and machines without Palworld installed stay green.
//!
//! Character counts use a broad sanity range because ordinary play changes them.

use std::path::PathBuf;
use std::time::Instant;

use paldex_gvas::{StructValue, Value};
use paldex_locate::{discover, resolve_manual};

fn real_level_sav() -> Option<PathBuf> {
    let root = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        resolve_manual(std::path::Path::new(&dir))
            .unwrap_or_else(|e| panic!("PALDEX_TEST_SAVE_DIR is set but unusable: {e}"))
            .into_iter()
            .next()
    } else {
        discover().into_iter().next()
    }?;

    let world = root.worlds().into_iter().find(|w| w.is_trackable())?;
    Some(world.path.join("Level.sav"))
}

macro_rules! require_real_level {
    () => {
        match real_level_sav() {
            Some(path) => path,
            None => {
                eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

fn character_map(root: &paldex_gvas::Root) -> Vec<(Value, Value)> {
    let Some(Value::Struct {
        value: StructValue::Properties(world),
        ..
    }) = root.get("worldSaveData")
    else {
        panic!("worldSaveData missing, or not the expected generic-struct shape");
    };
    let Some(prop) = world.iter().find(|p| p.name == "CharacterSaveParameterMap") else {
        panic!("CharacterSaveParameterMap missing from worldSaveData");
    };
    let Value::Map(entries) = &prop.value else {
        panic!("CharacterSaveParameterMap is not a Map, got {:?}", prop.value);
    };
    entries.clone()
}

#[test]
fn level_sav_parses_to_a_tree_containing_world_save_data() {
    let path = require_real_level!();
    let raw = std::fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();

    let root = paldex_gvas::parse(&gvas).unwrap();

    assert_eq!(root.header.engine_version.major, 5);
    assert_eq!(root.header.save_game_class_name, "/Script/Pal.PalWorldSaveGame");
    assert!(root.get("worldSaveData").is_some());
}

#[test]
fn character_save_parameter_map_has_a_plausible_entry_count() {
    let path = require_real_level!();
    let raw = std::fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let root = paldex_gvas::parse(&gvas).unwrap();

    let entries = character_map(&root);
    eprintln!("CharacterSaveParameterMap: {} entries", entries.len());

    // Sanity range, not an exact count — see the module doc. Zero would mean the
    // map parse silently produced nothing; a number in the hundreds of thousands
    // would mean something is looping instead of terminating.
    assert!(
        entries.len() > 100,
        "suspiciously few characters: {}",
        entries.len()
    );
    assert!(
        entries.len() < 1_000_000,
        "suspiciously many characters: {}",
        entries.len()
    );
}

#[test]
fn every_character_entry_decodes_as_a_struct_not_a_raw_fallback() {
    let path = require_real_level!();
    let raw = std::fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let root = paldex_gvas::parse(&gvas).unwrap();

    let entries = character_map(&root);
    let raw_fallbacks = entries
        .iter()
        .filter(|(_, v)| matches!(v, Value::Raw(_)))
        .count();

    assert_eq!(
        raw_fallbacks, 0,
        "{raw_fallbacks} of {} character entries fell back to Value::Raw — the map's \
         per-entry property-list parsing (verified against PlayerUId/InstanceId/DebugName \
         keys) desynced somewhere",
        entries.len()
    );
}

#[test]
fn every_character_carries_a_raw_data_blob_left_undecoded() {
    let path = require_real_level!();
    let raw = std::fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let root = paldex_gvas::parse(&gvas).unwrap();

    let entries = character_map(&root);
    for (_, value) in &entries {
        let Value::Struct {
            value: StructValue::Properties(fields),
            ..
        } = value
        else {
            panic!("expected every character entry to be a generic struct, got {value:?}");
        };
        let raw_data = fields.iter().find(|p| p.name == "RawData");
        assert!(
            matches!(raw_data.map(|p| &p.value), Some(Value::Raw(_))),
            "RawData missing or not left for the domain-specific decoder"
        );
    }
}

#[test]
fn level_sav_parses_well_under_the_two_second_budget() {
    let path = require_real_level!();
    let raw = std::fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();

    let start = Instant::now();
    paldex_gvas::parse(&gvas).unwrap();
    let elapsed = start.elapsed();

    eprintln!("parsed {} bytes in {elapsed:?}", gvas.len());
    assert!(
        elapsed.as_secs_f64() < 2.0,
        "parse took {elapsed:?}, over the 2s budget"
    );
}

#[test]
fn truncated_level_sav_errors_instead_of_panicking() {
    let path = require_real_level!();
    let raw = std::fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();

    // A sweep of truncation points, including mid-header and mid-property-list —
    // none of them should panic, whether they parse cleanly or return an error.
    for &frac in &[0.0001, 0.001, 0.01, 0.1, 0.5, 0.9, 0.999] {
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cut = ((gvas.len() as f64) * frac) as usize;
        let _ = paldex_gvas::parse(&gvas[..cut]);
    }
}
