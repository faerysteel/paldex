//! Decodes `BaseCampSaveData` entries into [`BaseCamp`]s.
//!
//! Like guilds (see the `guild` module), a base camp's top-level `RawData` is
//! Palworld's own bespoke binary serialization, not a GVAS property list —
//! confirmed by finding a real base's owning guild id (a known-good `Uuid`
//! from the `guild` decoder) sitting at a literal byte offset inside it,
//! alongside what looks like the base name as a length-prefixed UTF-16
//! string. Fully reverse-engineering that layout (worker assignments,
//! module/building data, HP, level) is real future work; for now this
//! extracts only the id and, best-effort, which guild owns it — enough to
//! satisfy the plan's "every base camp maps to an existing guild" check.

use uuid::Uuid;

use crate::gvas_ext::struct_properties;
use paldex_gvas::Value;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BaseCamp {
    pub id: Uuid,
    /// `None` if no id in `known_guild_ids` was found inside this base's raw
    /// bytes — could mean an unowned base, or just that the byte-scan missed
    /// it (this isn't a verified-exact field offset, see module docs).
    pub guild_id: Option<Uuid>,
}

/// Decode every entry of a `BaseCampSaveData`'s `Value::Map`, resolving
/// ownership against guild ids already decoded from `GroupSaveDataMap`
/// (see `decode_group_map`).
#[must_use]
pub fn decode_base_camp_map(entries: &[(Value, Value)], known_guild_ids: &[Uuid]) -> Vec<BaseCamp> {
    entries
        .iter()
        .filter_map(|(k, v)| decode_entry(k, v, known_guild_ids))
        .collect()
}

fn decode_entry(key: &Value, value: &Value, known_guild_ids: &[Uuid]) -> Option<BaseCamp> {
    let id = match key {
        Value::Struct {
            value: paldex_gvas::StructValue::Guid(bytes),
            ..
        } => Uuid::from_bytes(*bytes),
        _ => return None,
    };

    let props = struct_properties(value)?;
    let raw = props.iter().find_map(|p| match (&p.name[..], &p.value) {
        ("RawData", Value::Raw(bytes)) => Some(bytes),
        _ => None,
    });

    let guild_id = raw.and_then(|bytes| find_embedded_guid(bytes, known_guild_ids));

    Some(BaseCamp { id, guild_id })
}

fn find_embedded_guid(haystack: &[u8], candidates: &[Uuid]) -> Option<Uuid> {
    candidates
        .iter()
        .find(|id| haystack.windows(16).any(|w| w == id.as_bytes()))
        .copied()
}
