//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way the other crates' real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If
//! neither yields a trackable (hosted) world, every test here skips rather than
//! failing.

use paldex_gvas::{StructValue, Value};
use paldex_locate::{discover, resolve_manual};
use paldex_model::{decode_group_map, GroupKind};

fn group_map_entries() -> Option<Vec<(Value, Value)>> {
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
        .find(|p| p.name == "GroupSaveDataMap")
        .map(|p| &p.value)
    else {
        panic!("GroupSaveDataMap missing from worldSaveData");
    };
    Some(entries.clone())
}

macro_rules! require_real_groups {
    () => {
        match group_map_entries() {
            Some(entries) => entries,
            None => {
                eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

#[test]
fn every_group_decodes_with_a_valid_kind() {
    let entries = require_real_groups!();
    let guilds = decode_group_map(&entries);

    eprintln!(
        "{} groups: {} organization, {} guild, {} other",
        guilds.len(),
        guilds.iter().filter(|g| g.kind == GroupKind::Organization).count(),
        guilds.iter().filter(|g| g.kind == GroupKind::Guild).count(),
        guilds.iter().filter(|g| matches!(g.kind, GroupKind::Other(_))).count(),
    );

    assert_eq!(guilds.len(), entries.len(), "every group entry should decode");
    assert!(
        guilds.iter().any(|g| g.kind == GroupKind::Organization),
        "expected at least one auto-created Organization group"
    );
}

#[test]
fn at_least_one_guild_has_linked_character_ids() {
    let entries = require_real_groups!();
    let guilds = decode_group_map(&entries);

    let real_guilds: Vec<_> = guilds.iter().filter(|g| g.kind == GroupKind::Guild).collect();
    if real_guilds.is_empty() {
        eprintln!("skipping: no real (non-Organization) guild in this world");
        return;
    }

    let total_linked: usize = real_guilds.iter().map(|g| g.linked_character_ids.len()).sum();
    eprintln!("{} real guild(s), {total_linked} linked character id(s) total", real_guilds.len());
    assert!(total_linked > 0, "expected at least one linked character id in a real guild");
}
