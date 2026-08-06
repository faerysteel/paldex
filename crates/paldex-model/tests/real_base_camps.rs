//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way the other crates' real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If
//! neither yields a trackable (hosted) world, every test here skips rather than
//! failing.

use paldex_gvas::{StructValue, Value};
use paldex_locate::{discover, resolve_manual};
use paldex_model::{decode_base_camp_map, decode_group_map, GroupKind};

fn world_data() -> Option<Vec<paldex_gvas::Property>> {
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

    match parsed.get("worldSaveData") {
        Some(Value::Struct {
            value: StructValue::Properties(props),
            ..
        }) => Some(props.clone()),
        _ => panic!("worldSaveData missing or not the expected generic-struct shape"),
    }
}

fn map_entries(world: &[paldex_gvas::Property], field: &str) -> Vec<(Value, Value)> {
    match world.iter().find(|p| p.name == field).map(|p| &p.value) {
        Some(Value::Map(entries)) => entries.clone(),
        other => panic!("{field} missing or not a Map, got {other:?}"),
    }
}

macro_rules! require_world {
    () => {
        match world_data() {
            Some(w) => w,
            None => {
                eprintln!("skipping: no trackable Palworld world found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

#[test]
fn every_base_camp_decodes_and_most_resolve_to_a_guild() {
    let world = require_world!();
    let guild_ids: Vec<_> = decode_group_map(&map_entries(&world, "GroupSaveDataMap"))
        .into_iter()
        .filter(|g| g.kind == GroupKind::Guild)
        .map(|g| g.id)
        .collect();

    let base_entries = map_entries(&world, "BaseCampSaveData");
    let bases = decode_base_camp_map(&base_entries, &guild_ids);

    let resolved = bases.iter().filter(|b| b.guild_id.is_some()).count();
    eprintln!("{} base camps, {resolved} resolved to a guild", bases.len());

    assert_eq!(bases.len(), base_entries.len(), "every base camp entry should decode");
    if !guild_ids.is_empty() {
        assert!(resolved > 0, "expected at least one base camp to resolve to a real guild");
    }
}
