//! Dev-only: measure how well the real save's passive ids resolve against the
//! pak's skill name table, and show which prefix convention matches.
use std::collections::BTreeSet;
use std::path::Path;

use paldex_data::ReferenceData;
use paldex_gvas::{StructValue, Value};

fn main() {
    let pak_path = std::env::args().nth(1).expect("usage: check_passives <pak>");
    let mut pak = paldex_data::Pak::open(Path::new(&pak_path)).expect("open pak");
    let index = paldex_data::ReferenceIndex::extract(&mut pak, "en").expect("extract");

    let root = paldex_locate::discover().into_iter().next().expect("save root");
    let world = root.worlds().into_iter().find(paldex_locate::World::is_trackable).expect("world");
    let raw = std::fs::read(world.path.join("Level.sav")).expect("read Level.sav");
    let (gvas, _) = paldex_sav::decompress(&raw).expect("decompress");
    let parsed = paldex_gvas::parse(&gvas).expect("parse");

    let Some(Value::Struct { value: StructValue::Properties(world_data), .. }) =
        parsed.get("worldSaveData")
    else {
        panic!("worldSaveData missing");
    };
    let Some(Value::Map(entries)) =
        world_data.iter().find(|p| p.name == "CharacterSaveParameterMap").map(|p| &p.value)
    else {
        panic!("CharacterSaveParameterMap missing");
    };

    let result = paldex_model::decode_character_map(entries);
    let passives: BTreeSet<String> =
        result.pals.iter().flat_map(|p| p.passives.iter().cloned()).collect();
    println!("{} distinct passive ids in save", passives.len());

    for prefix in ["", "PASSIVE_"] {
        let hits = passives.iter().filter(|p| index.passive(&format!("{prefix}{p}")).is_some()).count();
        println!("  prefix {prefix:?}: {hits}/{} resolve", passives.len());
    }

    // Technologies come from the player save, not Level.sav; sample the ids
    // the store already surfaces instead.
    for id in ["COPPER", "Copper", "AssaultRifle_Default1"] {
        println!("tech {id:24} -> {:?}", index.technology(id));
    }

    println!("\n--- ids whose display name is unresolved or identical ---");
    let mut unresolved = 0;
    let mut identical = 0;
    for p in &passives {
        match index.passive(p).map(|s| s.display_name.as_str()) {
            None => {
                unresolved += 1;
                println!("  UNRESOLVED {p}");
            }
            Some(name) if name == p => {
                identical += 1;
                println!("  IDENTICAL  {p} -> {name:?}");
            }
            Some(_) => {}
        }
    }
    println!("{unresolved} unresolved, {identical} identical, {} renamed", passives.len() - unresolved - identical);

    // Instance-level counts, matching what the app test measures.
    let total: usize = result.pals.iter().map(|p| p.passives.len()).sum();
    let same: usize = result
        .pals
        .iter()
        .flat_map(|p| p.passives.iter())
        .filter(|id| index.passive(id).map(|s| s.display_name.as_str()) == Some(id.as_str()))
        .count();
    let missing: usize = result
        .pals
        .iter()
        .flat_map(|p| p.passives.iter())
        .filter(|id| index.passive(id).is_none())
        .count();
    println!("instances: {total} total, {missing} unresolved, {same} identical");
}
