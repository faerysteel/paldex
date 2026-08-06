//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way the other crates' real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If
//! neither yields a trackable (hosted) world, every test here skips rather than
//! failing, so CI and machines without Palworld installed stay green.

use paldex_gvas::{StructValue, Value};
use paldex_locate::{discover, resolve_manual};
use paldex_model::decode_character_map;

fn character_map_entries() -> Option<Vec<(Value, Value)>> {
    let root = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        resolve_manual(std::path::Path::new(&dir))
            .unwrap_or_else(|e| panic!("PALDEX_TEST_SAVE_DIR is set but unusable: {e}"))
            .into_iter()
            .next()
    } else {
        discover().into_iter().next()
    }?;

    let world = root.worlds().into_iter().find(|w| w.is_trackable())?;
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
    let Some(Value::Map(entries)) = world_data
        .iter()
        .find(|p| p.name == "CharacterSaveParameterMap")
        .map(|p| &p.value)
    else {
        panic!("CharacterSaveParameterMap missing from worldSaveData");
    };
    Some(entries.clone())
}

macro_rules! require_real_entries {
    () => {
        match character_map_entries() {
            Some(entries) => entries,
            None => {
                eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

#[test]
fn every_entry_classifies_with_no_warnings() {
    let entries = require_real_entries!();
    let result = decode_character_map(&entries);

    eprintln!(
        "{} pals, {} players, {} warnings",
        result.pals.len(),
        result.player_count,
        result.warnings.len()
    );
    for w in &result.warnings {
        eprintln!("  warning: {w}");
    }

    assert_eq!(
        result.warnings.len(),
        0,
        "every entry should classify as Pal or Player — see warnings above"
    );
    assert_eq!(
        result.pals.len() + result.player_count,
        entries.len(),
        "pals + players should account for every map entry"
    );
}

#[test]
fn every_iv_is_in_range() {
    let entries = require_real_entries!();
    let result = decode_character_map(&entries);

    for pal in &result.pals {
        // u8 is always <= 255, but the point is documenting + asserting the
        // domain invariant explicitly, the way the plan's success criteria ask.
        assert!(pal.ivs.hp <= 100, "{}: HP IV {} out of range", pal.character_id, pal.ivs.hp);
        assert!(
            pal.ivs.shot <= 100,
            "{}: Shot IV {} out of range",
            pal.character_id,
            pal.ivs.shot
        );
        assert!(
            pal.ivs.defense <= 100,
            "{}: Defense IV {} out of range",
            pal.character_id,
            pal.ivs.defense
        );
    }
}

#[test]
fn lucky_pal_count_is_plausible() {
    let entries = require_real_entries!();
    let result = decode_character_map(&entries);

    let lucky = result.pals.iter().filter(|p| p.is_lucky).count();
    eprintln!("lucky pals: {lucky} of {}", result.pals.len());
    // The plan's research pass found ~19 in an earlier snapshot of this same
    // world; natural play grows/shrinks this, so assert a sane range rather
    // than the stale exact figure.
    assert!(lucky < result.pals.len() / 10 + 5, "suspiciously many lucky pals: {lucky}");
}

#[test]
fn species_prefixes_are_stripped() {
    let entries = require_real_entries!();
    let result = decode_character_map(&entries);

    for pal in &result.pals {
        assert!(
            !pal.character_id.starts_with("BOSS_"),
            "BOSS_ prefix should be stripped, got {:?}",
            pal.character_id
        );
        assert!(
            !pal.character_id.starts_with("PREDATOR_"),
            "PREDATOR_ prefix should be stripped, got {:?}",
            pal.character_id
        );
    }
    assert!(
        result.pals.iter().any(|p| p.is_boss),
        "expected at least one BOSS_-prefixed pal in a world this size"
    );
}

#[test]
fn player_count_matches_a_real_player_character() {
    let entries = require_real_entries!();
    let result = decode_character_map(&entries);

    // At least the host has an in-world character entry.
    assert!(result.player_count >= 1);
}
