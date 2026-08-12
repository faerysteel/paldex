//! Debug dump: decode the real roster and print it as JSON, or a single named
//! Pal if a nickname/species filter is given. Manual-verification aid for
//! spot-checking against the in-game Pal detail screen.
//!
//! Usage: `cargo run -p paldex-model --example roster -- <Level.sav> [species-or-nickname-substring]`
use std::env;
use std::fs;

use paldex_gvas::{StructValue, Value};

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: roster <Level.sav> [filter]");
    let filter = args.next();

    let raw = fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let root = paldex_gvas::parse(&gvas).unwrap();

    let Some(Value::Struct {
        value: StructValue::Properties(world),
        ..
    }) = root.get("worldSaveData")
    else {
        panic!("no worldSaveData");
    };
    let Some(Value::Map(entries)) = world
        .iter()
        .find(|p| p.name == "CharacterSaveParameterMap")
        .map(|p| &p.value)
    else {
        panic!("no CharacterSaveParameterMap");
    };

    let result = paldex_model::decode_character_map(entries);
    eprintln!(
        "{} pals, {} players, {} warnings",
        result.pals.len(),
        result.player_count(),
        result.warnings.len()
    );

    let matches: Vec<_> = match &filter {
        Some(f) => result
            .pals
            .iter()
            .filter(|p| {
                p.character_id.to_lowercase().contains(&f.to_lowercase())
                    || p.nickname.as_deref().unwrap_or("").to_lowercase().contains(&f.to_lowercase())
            })
            .collect(),
        None => result.pals.iter().collect(),
    };

    println!("{}", serde_json::to_string_pretty(&matches).unwrap());
}
