//! Decodes `CharacterSaveParameterMap` entries into [`Pal`]s.
//!
//! Each map entry's key carries the `InstanceId`; the value carries a `RawData`
//! byte blob that is *itself* a nested GVAS property list containing a single
//! `SaveParameter` struct (`PalIndividualCharacterSaveParameter`) with the real
//! stats. See `paldex-gvas-rawdata-nesting` (memory) for how this was found.
//!
//! Players have an in-world character entry in this same map (`IsPlayer: true`)
//! sharing most of the same fields — these are skipped from the roster; their
//! real progression data lives in a separate `Players/<uid>.sav` file, not
//! decoded here.

use paldex_gvas::{ByteValue, Property, StructValue, Value};
use uuid::Uuid;

use crate::types::{Gender, Ivs, PalLocation, Pal, SoulUpgrades};

/// The result of decoding `CharacterSaveParameterMap`.
///
/// Never fails outright: an entry that can't be classified or decoded is
/// recorded in `warnings` and skipped, rather than aborting the whole map —
/// the same "warn, don't fail" philosophy `paldex-gvas` uses for `Value::Raw`.
#[derive(Debug, Default)]
pub struct CharacterMapResult {
    pub pals: Vec<Pal>,
    /// Count of entries classified as the player's own in-world character
    /// (`IsPlayer: true`). Not included in `pals` — see module docs.
    pub player_count: usize,
    pub warnings: Vec<String>,
}

/// Decode every entry of a `CharacterSaveParameterMap`'s `Value::Map`.
#[must_use]
pub fn decode_character_map(entries: &[(Value, Value)]) -> CharacterMapResult {
    let mut result = CharacterMapResult::default();

    for (key, value) in entries {
        match decode_entry(key, value) {
            Ok(Classified::Pal(pal)) => result.pals.push(pal),
            Ok(Classified::Player) => result.player_count += 1,
            Err(reason) => result.warnings.push(reason),
        }
    }

    result
}

enum Classified {
    Pal(Pal),
    Player,
}

fn decode_entry(key: &Value, value: &Value) -> Result<Classified, String> {
    let key_props = struct_properties(key).ok_or("map key is not a generic struct")?;
    let instance_id = find_guid(key_props, "InstanceId").ok_or("key missing InstanceId")?;

    let value_props = struct_properties(value).ok_or("map value is not a generic struct")?;
    let raw_data = match find(value_props, "RawData") {
        Some(Value::Raw(bytes)) => bytes,
        _ => return Err(format!("entry {instance_id} missing RawData")),
    };

    let inner = paldex_gvas::parse_property_list_bytes(raw_data)
        .map_err(|e| format!("entry {instance_id}: RawData did not parse as a property list: {e}"))?;
    let save_param = match find(&inner, "SaveParameter") {
        Some(Value::Struct {
            value: StructValue::Properties(sp),
            ..
        }) => sp,
        _ => return Err(format!("entry {instance_id} missing SaveParameter")),
    };

    if matches!(find(save_param, "IsPlayer"), Some(Value::Bool(true))) {
        return Ok(Classified::Player);
    }

    Ok(Classified::Pal(build_pal(instance_id, save_param)))
}

fn build_pal(instance_id: Uuid, sp: &[Property]) -> Pal {
    let raw_character_id = match find(sp, "CharacterID") {
        Some(Value::Name(s)) => s.as_str(),
        _ => "",
    };
    let (character_id, is_boss, is_predator) = strip_species_prefix(raw_character_id);

    Pal {
        instance_id,
        character_id,
        owner: find_guid(sp, "OwnerPlayerUId").filter(|g| !g.is_nil()),
        level: find_byte(sp, "Level").unwrap_or(1),
        rank: find_byte(sp, "Rank").unwrap_or(1),
        souls: SoulUpgrades {
            hp: find_byte(sp, "Rank_HP").unwrap_or(0),
            attack: find_byte(sp, "Rank_Attack").unwrap_or(0),
            defense: find_byte(sp, "Rank_Defence").unwrap_or(0),
            craft_speed: find_byte(sp, "Rank_CraftSpeed").unwrap_or(0),
        },
        ivs: Ivs {
            hp: find_byte(sp, "Talent_HP").unwrap_or(0),
            shot: find_byte(sp, "Talent_Shot").unwrap_or(0),
            defense: find_byte(sp, "Talent_Defense").unwrap_or(0),
        },
        passives: find_name_array(sp, "PassiveSkillList"),
        equipped_moves: find_enum_array(sp, "EquipWaza"),
        mastered_moves: find_enum_array(sp, "MasteredWaza"),
        gender: find_enum(sp, "Gender").map_or(Gender::Unknown, parse_gender),
        is_lucky: matches!(find(sp, "IsRarePal"), Some(Value::Bool(true))),
        is_boss,
        is_predator,
        nickname: find_str(sp, "NickName").filter(|s| !s.is_empty()).map(str::to_owned),
        location: find_slot_id(sp),
    }
}

fn strip_species_prefix(id: &str) -> (String, bool, bool) {
    if let Some(rest) = id.strip_prefix("BOSS_") {
        (rest.to_owned(), true, false)
    } else if let Some(rest) = id.strip_prefix("PREDATOR_") {
        (rest.to_owned(), false, true)
    } else {
        (id.to_owned(), false, false)
    }
}

fn parse_gender(value: &str) -> Gender {
    match value.rsplit("::").next().unwrap_or(value) {
        "Male" => Gender::Male,
        "Female" => Gender::Female,
        _ => Gender::Unknown,
    }
}

fn find<'a>(props: &'a [Property], name: &str) -> Option<&'a Value> {
    props.iter().find(|p| p.name == name).map(|p| &p.value)
}

fn struct_properties(value: &Value) -> Option<&[Property]> {
    match value {
        Value::Struct {
            value: StructValue::Properties(props),
            ..
        } => Some(props),
        _ => None,
    }
}

fn find_guid(props: &[Property], name: &str) -> Option<Uuid> {
    match find(props, name) {
        Some(Value::Struct {
            value: StructValue::Guid(bytes),
            ..
        }) => Some(Uuid::from_bytes(*bytes)),
        _ => None,
    }
}

fn find_byte(props: &[Property], name: &str) -> Option<u8> {
    match find(props, name) {
        Some(Value::Byte {
            value: ByteValue::Raw(b),
            ..
        }) => Some(*b),
        _ => None,
    }
}

fn find_str<'a>(props: &'a [Property], name: &str) -> Option<&'a str> {
    match find(props, name) {
        Some(Value::Str(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn find_enum<'a>(props: &'a [Property], name: &str) -> Option<&'a str> {
    match find(props, name) {
        Some(Value::Enum { value, .. }) => Some(value.as_str()),
        _ => None,
    }
}

fn find_name_array(props: &[Property], name: &str) -> Vec<String> {
    match find(props, name) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Name(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn find_enum_array(props: &[Property], name: &str) -> Vec<String> {
    match find(props, name) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Enum { value, .. } => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn find_slot_id(sp: &[Property]) -> Option<PalLocation> {
    let slot_props = struct_properties(find(sp, "SlotId")?)?;
    let container_props = struct_properties(find(slot_props, "ContainerId")?)?;
    let container_id = find_guid(container_props, "ID")?;
    let slot_index = match find(slot_props, "SlotIndex") {
        Some(Value::Int(n)) => u32::try_from(*n).ok()?,
        _ => return None,
    };
    Some(PalLocation {
        container_id,
        slot_index,
    })
}
