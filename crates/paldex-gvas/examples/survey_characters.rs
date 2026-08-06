//! Dev-only: survey field names/types across every character in a real
//! `Level.sav`, and print one full non-player record for close inspection.
use std::collections::BTreeMap;
use std::env;
use std::fs;

use paldex_gvas::{StructValue, Value};

fn kind_tag(v: &Value) -> String {
    match v {
        Value::Int(_) => "Int".into(),
        Value::Int64(_) => "Int64".into(),
        Value::UInt32(_) => "UInt32".into(),
        Value::Float(_) => "Float".into(),
        Value::Double(_) => "Double".into(),
        Value::Bool(_) => "Bool".into(),
        Value::Str(_) => "Str".into(),
        Value::Name(_) => "Name".into(),
        Value::Enum { enum_type, .. } => format!("Enum({enum_type})"),
        Value::Byte { enum_type, .. } => format!("Byte(enum={enum_type:?})"),
        Value::Struct { struct_name, value } => format!(
            "Struct({struct_name}, {})",
            match value {
                StructValue::Guid(_) => "Guid",
                StructValue::DateTime(_) => "DateTime",
                StructValue::Properties(_) => "Properties",
                StructValue::Raw(_) => "Raw",
            }
        ),
        Value::Array(items) => {
            let inner = items.first().map(kind_tag).unwrap_or_default();
            format!("Array<{inner}>[{}]", items.len())
        }
        Value::Map(_) => "Map".into(),
        Value::Set(_) => "Set".into(),
        Value::Raw(b) => format!("Raw({}B)", b.len()),
    }
}

fn main() {
    let path = env::args().nth(1).expect("usage: survey_characters <Level.sav>");
    let raw = fs::read(&path).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let root = paldex_gvas::parse(&gvas).unwrap();

    let Some(Value::Struct { value: StructValue::Properties(world), .. }) = root.get("worldSaveData") else {
        panic!("no worldSaveData");
    };
    let Some(Value::Map(entries)) = world.iter().find(|p| p.name == "CharacterSaveParameterMap").map(|p| &p.value) else {
        panic!("no CharacterSaveParameterMap");
    };

    let mut field_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut sample_non_player: Option<serde_json::Value> = None;
    let mut is_player_count = 0usize;
    let mut character_ids: BTreeMap<String, usize> = BTreeMap::new();
    // (has_talent_count, no_talent_count) per CharacterID
    let mut talent_by_id: BTreeMap<String, (usize, usize)> = BTreeMap::new();

    for (_, v) in entries {
        let Value::Struct { value: StructValue::Properties(fields), .. } = v else { continue };
        let Some(raw_data) = fields.iter().find(|p| p.name == "RawData") else { continue };
        let Value::Raw(bytes) = &raw_data.value else { continue };
        let Ok(inner) = paldex_gvas::parse_property_list_bytes(bytes) else { continue };
        let Some(save_param) = inner.iter().find(|p| p.name == "SaveParameter") else { continue };
        let Value::Struct { value: StructValue::Properties(sp), .. } = &save_param.value else { continue };

        let mut is_player = false;
        let mut char_id = None;
        let has_talent = sp.iter().any(|f| f.name == "Talent_HP" || f.name == "Talent_Shot" || f.name == "Talent_Defense");
        for f in sp {
            *field_counts.entry(f.name.clone()).or_default() += 1;
            if f.name == "IsPlayer" {
                if let Value::Bool(true) = f.value {
                    is_player = true;
                }
            }
            if f.name == "CharacterID" {
                if let Value::Name(s) = &f.value {
                    char_id = Some(s.clone());
                }
            }
        }
        if is_player {
            is_player_count += 1;
        }
        if let Some(id) = &char_id {
            *character_ids.entry(id.clone()).or_default() += 1;
        }
        if !is_player {
            if let Some(id) = &char_id {
                let entry = talent_by_id.entry(id.clone()).or_default();
                if has_talent {
                    entry.0 += 1;
                } else {
                    entry.1 += 1;
                }
            }
        }
        if !is_player && sample_non_player.is_none() {
            let slot = sp.iter().find(|f| f.name == "SlotId").map(|f| serde_json::to_value(&f.value).unwrap());
            let owner = sp.iter().find(|f| f.name == "OwnerPlayerUId").map(|f| serde_json::to_value(&f.value).unwrap());
            println!("sample SlotId: {}", serde_json::to_string_pretty(&slot).unwrap());
            println!("sample OwnerPlayerUId: {}", serde_json::to_string_pretty(&owner).unwrap());
            sample_non_player = Some(serde_json::to_value(sp).unwrap());
        }
    }

    let watch = [
        "CharacterID", "Gender", "Level", "Talent_HP", "Talent_Shot", "Talent_Defense",
        "EquipWaza", "MasteredWaza", "PassiveSkillList", "OwnerPlayerUId", "Rank",
        "Rank_HP", "Rank_Attack", "Rank_Defence", "Rank_CraftSpeed", "IsRarePal",
        "NickName", "SlotId", "IsPlayer",
    ];
    let mut kinds: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
    for (_, v) in entries {
        let Value::Struct { value: StructValue::Properties(fields), .. } = v else { continue };
        let Some(raw_data) = fields.iter().find(|p| p.name == "RawData") else { continue };
        let Value::Raw(bytes) = &raw_data.value else { continue };
        let Ok(inner) = paldex_gvas::parse_property_list_bytes(bytes) else { continue };
        let Some(save_param) = inner.iter().find(|p| p.name == "SaveParameter") else { continue };
        let Value::Struct { value: StructValue::Properties(sp), .. } = &save_param.value else { continue };
        for f in sp {
            if let Some(&name) = watch.iter().find(|&&w| w == f.name) {
                *kinds.entry(name).or_default().entry(kind_tag(&f.value)).or_default() += 1;
            }
        }
    }
    println!("watched field -> value kind counts:");
    for (name, tally) in &kinds {
        println!("  {name}: {tally:?}");
    }
    println!();

    println!("total entries: {}", entries.len());
    println!("IsPlayer=true: {is_player_count}");
    println!("distinct CharacterID: {}", character_ids.len());
    println!("\nfield occurrence counts:");
    for (name, count) in &field_counts {
        println!("  {name}: {count}");
    }
    let never_has_talent: Vec<_> = talent_by_id.iter().filter(|(_, (has, _))| *has == 0).collect();
    let always_has_talent: usize = talent_by_id.iter().filter(|(_, (_, no))| *no == 0).count();
    let mixed: Vec<_> = talent_by_id.iter().filter(|(_, (has, no))| *has > 0 && *no > 0).collect();
    println!("\nCharacterIDs that NEVER have Talent_* ({}):", never_has_talent.len());
    for (id, (has, no)) in &never_has_talent {
        println!("  {id}: has={has} no={no}");
    }
    println!("\nCharacterIDs ALWAYS have Talent_* (species count): {always_has_talent}");
    println!("\nCharacterIDs with MIXED talent presence ({}):", mixed.len());
    for (id, (has, no)) in &mixed {
        println!("  {id}: has={has} no={no}");
    }
    println!("\nsample non-player record:");
    println!("{}", serde_json::to_string_pretty(&sample_non_player).unwrap());
}
