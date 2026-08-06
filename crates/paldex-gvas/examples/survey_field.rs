//! Dev-only: survey a single worldSaveData field's shape/type against a real
//! `Level.sav`, printing counts and one sample decoded JSON.
use std::env;
use std::fs;

use paldex_gvas::{StructValue, Value};

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("usage: survey_field <Level.sav> <field-name> [sample-count]");
    let field = args.next().expect("usage: survey_field <Level.sav> <field-name> [sample-count]");
    let sample_count: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let raw = fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let root = paldex_gvas::parse(&gvas).unwrap();

    let Some(Value::Struct { value: StructValue::Properties(world), .. }) = root.get("worldSaveData") else {
        panic!("no worldSaveData");
    };
    let Some(prop) = world.iter().find(|p| p.name == field) else {
        panic!("field {field:?} not found; available: {:?}", world.iter().map(|p| &p.name).collect::<Vec<_>>());
    };

    match &prop.value {
        Value::Map(entries) => {
            println!("{field}: Map with {} entries", entries.len());
            for (k, v) in entries.iter().take(sample_count) {
                println!("--- key ---\n{}", serde_json::to_string_pretty(k).unwrap());
                println!("--- value ---\n{}", serde_json::to_string_pretty(v).unwrap());
            }
        }
        Value::Array(items) => {
            println!("{field}: Array with {} items", items.len());
            for v in items.iter().take(sample_count) {
                println!("--- item ---\n{}", serde_json::to_string_pretty(v).unwrap());
            }
        }
        Value::Raw(bytes) => {
            println!("{field}: Raw({} bytes) — attempting to re-parse as a property list", bytes.len());
            match paldex_gvas::parse_property_list_bytes(bytes) {
                Ok(props) => println!("{}", serde_json::to_string_pretty(&props).unwrap()),
                Err(e) => println!("failed to parse as property list: {e}\nfirst 128 bytes: {:02x?}", &bytes[..bytes.len().min(128)]),
            }
        }
        other => {
            println!("{field}: {}", serde_json::to_string_pretty(other).unwrap());
        }
    }
}
