//! Decodes `GroupSaveDataMap` entries into [`Guild`]s.
//!
//! Unlike `CharacterSaveParameterMap`'s `RawData` (a nested GVAS property
//! list, see the `character` module), a guild's `RawData` is a bespoke binary
//! layout — Palworld's own hand-rolled `Serialize()`, not the standard
//! tagged-property format. `paldex_gvas::parse_property_list_bytes` fails on
//! it outright.
//!
//! Only a best-effort subset is decoded here, verified against a real save
//! via `crates/paldex-gvas/examples/survey_field.rs`:
//! - The map key's `Guid` is the group's own id.
//! - `GroupType` (`EPalGroupType::Organization` vs `::Guild`) — every player
//!   gets an auto-created `Organization` entry; `Guild` entries are the real
//!   multiplayer guilds.
//! - Linked ids: verified by finding a real player's own `IndividualId.InstanceId`
//!   (from `Players/<uid>.sav`) inside a `Guild`-type `RawData` blob, each
//!   occurrence preceded by 12 zero bytes then a `1u32` marker. Scanning for
//!   that exact marker pattern is more robust than assuming a precise fixed
//!   byte layout for a format that was never fully reverse-engineered.
//!   Against a real single-guild world, the extracted count (1976) landed
//!   almost exactly on the total character count (1973 pals + 2 players +
//!   the guild's own admin entry) — so this is most likely every character
//!   *owned by* the guild (pals included), not a roster of human members.
//!   The full struct almost certainly carries more (guild name, base
//!   ownership, an actual member-vs-owned-pal distinction) that this
//!   doesn't extract yet.

use uuid::Uuid;

use crate::gvas_ext::{find, find_enum, struct_properties};
use paldex_gvas::Value;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum GroupKind {
    /// Auto-created per player; not a real multiplayer guild.
    Organization,
    Guild,
    /// Any `EPalGroupType` value other than the two above.
    Other(String),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Guild {
    pub id: Uuid,
    pub kind: GroupKind,
    /// Best-effort, `GroupKind::Guild` only — see module docs. Empty for
    /// `Organization`/`Other` groups, and possibly incomplete even for real
    /// guilds, since the byte layout beyond this marker isn't fully decoded.
    pub linked_character_ids: Vec<Uuid>,
}

/// The 4-byte marker (`1u32`, little-endian) verified to immediately precede
/// each linked character id in a real `Guild`-type `RawData` blob.
const LINKED_ID_MARKER: [u8; 4] = 1u32.to_le_bytes();

/// Decode every entry of a `GroupSaveDataMap`'s `Value::Map`.
#[must_use]
pub fn decode_group_map(entries: &[(Value, Value)]) -> Vec<Guild> {
    entries.iter().filter_map(|(k, v)| decode_entry(k, v)).collect()
}

fn decode_entry(key: &Value, value: &Value) -> Option<Guild> {
    let id = find_container_id_from_guid_key(key)?;
    let value_props = struct_properties(value)?;
    let kind = match find_enum(value_props, "GroupType") {
        Some(s) if s.ends_with("::Organization") => GroupKind::Organization,
        Some(s) if s.ends_with("::Guild") => GroupKind::Guild,
        Some(other) => GroupKind::Other(other.to_owned()),
        None => GroupKind::Other(String::new()),
    };

    let linked_character_ids = if kind == GroupKind::Guild {
        match find(value_props, "RawData") {
            Some(Value::Raw(bytes)) => scan_linked_character_ids(bytes),
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    Some(Guild { id, kind, linked_character_ids })
}

/// The map key here is a bare `Guid` (see `paldex-gvas`'s
/// `decode_struct_collection_element` heuristic), not the `PalContainerId`-
/// wrapped shape `find_container_id` expects — unwrap it directly instead.
fn find_container_id_from_guid_key(key: &Value) -> Option<Uuid> {
    match key {
        Value::Struct {
            value: paldex_gvas::StructValue::Guid(bytes),
            ..
        } => Some(Uuid::from_bytes(*bytes)),
        _ => None,
    }
}

fn scan_linked_character_ids(bytes: &[u8]) -> Vec<Uuid> {
    let mut ids = Vec::new();
    let mut i = 0;
    while i + 4 + 16 <= bytes.len() {
        if bytes[i..i + 4] == LINKED_ID_MARKER {
            let mut guid = [0u8; 16];
            guid.copy_from_slice(&bytes[i + 4..i + 20]);
            ids.push(Uuid::from_bytes(guid));
            i += 20;
        } else {
            i += 1;
        }
    }
    ids
}
