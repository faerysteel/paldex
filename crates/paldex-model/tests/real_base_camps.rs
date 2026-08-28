//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way the other crates' real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If
//! neither yields a trackable (hosted) world, every test here skips rather than
//! failing.

use std::collections::HashMap;

use paldex_gvas::{StructValue, Value};
use paldex_locate::{discover, resolve_manual};
use paldex_model::{decode_base_camp_map, decode_character_map, decode_group_map, GroupKind};
use uuid::Uuid;

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

/// The offset-0 self-check. `worker_container` refuses to slice a blob whose
/// leading 16 bytes aren't the map key, so a decoded `worker_container_id` is
/// itself the assertion that the layout hasn't moved — and if it has, every
/// other number in this file is suspect.
#[test]
fn every_base_camps_worker_director_starts_with_its_own_id() {
    let world = require_world!();
    let base_entries = map_entries(&world, "BaseCampSaveData");
    let bases = decode_base_camp_map(&base_entries, &[]);

    let missing: Vec<_> = bases
        .iter()
        .filter(|b| b.worker_container_id.is_none())
        .map(|b| b.id)
        .collect();

    assert!(
        missing.is_empty(),
        "{} of {} bases failed the WorkerDirector length/self-id check — the 118-byte \
         layout in base_camp.rs's module docs has moved: {missing:?}",
        missing.len(),
        bases.len(),
    );
}

/// The join the whole Bases tab rests on: every Pal belonging to no player
/// sits in exactly one base's worker container, and no two bases claim the
/// same one.
///
/// Asserts the *partition* rather than the literal counts, so the test stays
/// meaningful after the save changes.
#[test]
fn every_unowned_pal_belongs_to_exactly_one_bases_worker_container() {
    let world = require_world!();
    let bases = decode_base_camp_map(&map_entries(&world, "BaseCampSaveData"), &[]);
    let pals = decode_character_map(&map_entries(&world, "CharacterSaveParameterMap")).pals;

    let containers: Vec<(Uuid, Uuid)> = bases
        .iter()
        .filter_map(|b| b.worker_container_id.map(|c| (b.id, c)))
        .collect();

    let mut owning_base: HashMap<Uuid, Uuid> = HashMap::new();
    for (base_id, container) in &containers {
        if let Some(other) = owning_base.insert(*container, *base_id) {
            panic!("container {container} is claimed by both base {other} and base {base_id}");
        }
    }

    let unowned: Vec<_> = pals.iter().filter(|p| p.owner.is_none()).collect();
    let mut per_base: HashMap<Uuid, usize> = containers.iter().map(|(id, _)| (*id, 0)).collect();
    let mut unmatched = 0usize;

    for pal in &unowned {
        match pal.location.and_then(|l| owning_base.get(&l.container_id).copied()) {
            Some(base_id) => *per_base.entry(base_id).or_default() += 1,
            None => unmatched += 1,
        }
    }

    let mut counts: Vec<_> = per_base.iter().map(|(id, n)| (*id, *n)).collect();
    counts.sort_by_key(|(id, _)| *id);
    for (id, n) in &counts {
        eprintln!("base {id}: {n} workers");
    }
    eprintln!(
        "{} unowned pals, {} matched to a base, {unmatched} unmatched",
        unowned.len(),
        unowned.len() - unmatched,
    );

    assert_eq!(
        containers.len(),
        bases.len(),
        "every base should have decoded a worker container",
    );
    assert_eq!(
        unmatched, 0,
        "every Pal belonging to no player should work at some base",
    );
}
