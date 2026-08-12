//! Decodes `CharacterSaveParameterMap` entries into [`Pal`]s.
//!
//! Each map entry's key carries the `InstanceId`; the value carries a `RawData`
//! byte blob that is *itself* a nested GVAS property list containing a single
//! `SaveParameter` struct (`PalIndividualCharacterSaveParameter`) with the real
//! stats. See `paldex-gvas-rawdata-nesting` (memory) for how this was found.
//!
//! Players have an in-world character entry in this same map (`IsPlayer: true`)
//! sharing most of the same fields — these are kept out of the roster, but
//! they are the only place the player's *name* is written down, so they are
//! decoded into [`PlayerIdentity`] rather than merely counted. Their real
//! progression data lives in a separate `Players/<uid>.sav` file, decoded by
//! the `player` module; the `PlayerUId` on this map's key is the same value
//! that file's `SaveData.PlayerUId` carries, which is what lets the two be
//! joined.

use paldex_gvas::{StructValue, Value};
use uuid::Uuid;

use crate::gvas_ext::{
    find, find_byte, find_container_id, find_enum, find_enum_array, find_guid, find_name_array,
    find_str, struct_properties,
};
use crate::types::{Gender, Ivs, Pal, PalLocation, PalLocationKind, PlayerIdentity, SoulUpgrades};

/// The result of decoding `CharacterSaveParameterMap`.
///
/// Never fails outright: an entry that can't be classified or decoded is
/// recorded in `warnings` and skipped, rather than aborting the whole map —
/// the same "warn, don't fail" philosophy `paldex-gvas` uses for `Value::Raw`.
#[derive(Debug, Default)]
pub struct CharacterMapResult {
    pub pals: Vec<Pal>,
    /// Entries classified as a player's own in-world character
    /// (`IsPlayer: true`). Not included in `pals` — see module docs.
    pub players: Vec<PlayerIdentity>,
    pub warnings: Vec<String>,
}

impl CharacterMapResult {
    /// How many player characters this map held.
    #[must_use]
    pub fn player_count(&self) -> usize {
        self.players.len()
    }
}

/// Decode every entry of a `CharacterSaveParameterMap`'s `Value::Map`.
#[must_use]
pub fn decode_character_map(entries: &[(Value, Value)]) -> CharacterMapResult {
    let mut result = CharacterMapResult::default();

    for (key, value) in entries {
        match decode_entry(key, value) {
            Ok(Classified::Pal(pal)) => result.pals.push(pal),
            Ok(Classified::Player(identity)) => result.players.push(identity),
            Err(reason) => result.warnings.push(reason),
        }
    }

    result
}

enum Classified {
    Pal(Pal),
    Player(PlayerIdentity),
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
        // The uid lives on the map *key*, not in `SaveParameter` — the value
        // side has no field naming which player this character is.
        let uid = find_guid(key_props, "PlayerUId")
            .filter(|g| !g.is_nil())
            .ok_or_else(|| format!("player entry {instance_id} has no PlayerUId on its key"))?;
        return Ok(Classified::Player(PlayerIdentity {
            uid,
            instance_id,
            // `FilteredNickName` is the profanity-filtered variant; the raw
            // `NickName` is what the player set and what the game shows them.
            name: find_str(save_param, "NickName")
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            level: find_byte(save_param, "Level").unwrap_or(1),
        }));
    }

    Ok(Classified::Pal(build_pal(instance_id, save_param)))
}

fn build_pal(instance_id: Uuid, sp: &[paldex_gvas::Property]) -> Pal {
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

fn find_slot_id(sp: &[paldex_gvas::Property]) -> Option<PalLocation> {
    let slot_props = struct_properties(find(sp, "SlotId")?)?;
    let container_id = find_container_id(slot_props, "ContainerId")?;
    let slot_index = match find(slot_props, "SlotIndex") {
        Some(Value::Int(n)) => u32::try_from(*n).ok()?,
        _ => return None,
    };
    Some(PalLocation {
        container_id,
        slot_index,
        // Resolving Party/Box requires a player's container IDs, which live in
        // a different file (`Players/<uid>.sav`) — see `resolve_locations`.
        kind: PalLocationKind::Other,
    })
}
