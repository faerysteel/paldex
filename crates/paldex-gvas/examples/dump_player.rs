//! Dump a `Players/<uid>.sav`: root and `SaveData` field names, selected values,
//! `RecordData` field types, and sample map entries.
//!
//! Usage: `cargo run -p paldex-gvas --example dump_player -- <Players/uid.sav>`
use std::env;
use std::fs;

use paldex_gvas::{StructValue, Value};

fn main() {
    let path = env::args().nth(1).expect("usage: dump_player <Players/uid.sav>");
    let raw = fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let root = paldex_gvas::parse(&gvas).unwrap();

    println!("top-level properties: {:?}", root.properties.iter().map(|p| &p.name).collect::<Vec<_>>());

    let Some(Value::Struct { value: StructValue::Properties(save_data), .. }) =
        root.properties.iter().find(|p| p.name == "SaveData").map(|p| &p.value)
    else {
        println!("no SaveData; full dump:");
        println!("{}", serde_json::to_string_pretty(&root).unwrap());
        return;
    };
    println!("SaveData fields: {:?}", save_data.iter().map(|p| &p.name).collect::<Vec<_>>());
    for name in [
        "OtomoCharacterContainerId",
        "PalStorageContainerId",
        "PlayerUId",
        "IndividualId",
        "TechnologyPoint",
        "bossTechnologyPoint",
        "UnlockedRecipeTechnologyNames",
        "OrderedQuestArray_FullRelease",
        "CompletedQuestArray_FullRelease",
    ] {
        if let Some(p) = save_data.iter().find(|p| p.name == name) {
            println!("{name}: {}", kind(&p.value));
            println!("  {}", serde_json::to_string_pretty(&p.value).unwrap());
        }
    }

    if let Some(record) = save_data.iter().find(|p| p.name == "RecordData") {
        if let Value::Struct { value: StructValue::Properties(rd), .. } = &record.value {
            println!("RecordData fields ({}):", rd.len());
            for f in rd {
                println!("  {}: {}", f.name, kind(&f.value));
            }
            for name in ["PaldeckUnlockFlag", "PalCaptureCount", "TribeCaptureCount"] {
                if let Some(f) = rd.iter().find(|f| f.name == name) {
                    println!("\n{name} sample:");
                    match &f.value {
                        Value::Map(entries) => {
                            for (k, v) in entries.iter().take(3) {
                                println!("  key={} value={}", kind(k), kind(v));
                            }
                        }
                        other => println!("  {}", serde_json::to_string_pretty(other).unwrap()),
                    }
                }
            }
        } else {
            println!("RecordData: {:?}", record.value);
        }
    } else {
        println!("no RecordData field");
    }
}

fn kind(v: &Value) -> String {
    match v {
        Value::Int(n) => format!("Int({n})"),
        Value::Int64(n) => format!("Int64({n})"),
        Value::UInt32(n) => format!("UInt32({n})"),
        Value::Float(n) => format!("Float({n})"),
        Value::Double(n) => format!("Double({n})"),
        Value::Bool(b) => format!("Bool({b})"),
        Value::Str(s) => format!("Str({s:?})"),
        Value::Name(s) => format!("Name({s:?})"),
        Value::Enum { enum_type, value } => format!("Enum({enum_type}={value})"),
        Value::Byte { .. } => "Byte".into(),
        Value::Struct { struct_name, value } => format!(
            "Struct({struct_name}, {})",
            match value {
                StructValue::Guid(_) => "Guid".to_string(),
                StructValue::DateTime(_) => "DateTime".to_string(),
                StructValue::Properties(p) => format!("Properties[{}]", p.len()),
                StructValue::Raw(b) => format!("Raw({}B)", b.len()),
            }
        ),
        Value::Array(items) => format!("Array[{}]", items.len()),
        Value::Map(entries) => format!("Map[{}]", entries.len()),
        Value::Set(items) => format!("Set[{}]", items.len()),
        Value::Raw(b) => format!("Raw({}B)", b.len()),
    }
}
