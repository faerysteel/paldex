//! Debug dump: decompress and parse a real `.sav` file, printing it as JSON.
//!
//! This is the manual-verification aid called for in Phase 0's plan: something to
//! eyeball a save's parsed shape against what the game itself reports. For
//! `Level.sav` specifically — too large to dump in full — it prints the top-level
//! property names plus one fully-decoded character record, re-parsing that
//! character's `RawData` blob (opaque to the main parser by design; see the
//! `value` module docs) so there's something concrete to spot-check a Pal's
//! species/level/IVs against what's shown in-game.
//!
//! Usage: `cargo run -p paldex-gvas --example dump -- <path/to/save.sav>`
use std::env;
use std::fs;

use paldex_gvas::{StructValue, Value};

fn main() {
    let path = env::args().nth(1).expect("usage: dump <path/to/save.sav>");
    let raw = fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    let (gvas, _compression) = paldex_sav::decompress(&raw).expect("decompress");
    let root = paldex_gvas::parse(&gvas).expect("parse");

    let world = root.get("worldSaveData").and_then(|v| match v {
        Value::Struct {
            value: StructValue::Properties(props),
            ..
        } => Some(props),
        _ => None,
    });

    let Some(world) = world else {
        // Not a world save (e.g. a player .sav) — the whole tree is small
        // enough to just dump directly.
        println!("{}", serde_json::to_string_pretty(&root).unwrap());
        return;
    };

    println!(
        "top-level properties: {:?}",
        root.properties.iter().map(|p| &p.name).collect::<Vec<_>>()
    );
    println!(
        "worldSaveData fields: {:?}",
        world.iter().map(|p| &p.name).collect::<Vec<_>>()
    );

    let Some(char_map) = world
        .iter()
        .find(|p| p.name == "CharacterSaveParameterMap")
        .and_then(|p| match &p.value {
            Value::Map(entries) => Some(entries),
            _ => None,
        })
    else {
        return;
    };
    println!("CharacterSaveParameterMap entries: {}", char_map.len());

    let Some((_, Value::Struct {
        value: StructValue::Properties(fields),
        ..
    })) = char_map.first()
    else {
        return;
    };
    let Some(raw_data) = fields
        .iter()
        .find(|p| p.name == "RawData")
        .and_then(|p| match &p.value {
            Value::Raw(bytes) => Some(bytes),
            _ => None,
        })
    else {
        return;
    };

    let character = paldex_gvas::parse_property_list_bytes(raw_data)
        .expect("RawData should itself be a property list");
    println!("\nfirst character record (decoded RawData):");
    println!("{}", serde_json::to_string_pretty(&character).unwrap());
}
