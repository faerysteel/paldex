//! Decompress a `.sav` and print decoded data.
//!
//! World saves: property names, character count, and the first character's
//! reparsed `RawData` as JSON. Other saves: the full property tree as JSON.
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
