//! Decodes `BaseCampSaveData` entries into [`BaseCamp`]s.
//!
//! Like guilds (see the `guild` module), a base camp's top-level `RawData` is
//! Palworld's own bespoke binary serialization, not a GVAS property list —
//! confirmed by finding a real base's owning guild id (a known-good `Uuid`
//! from the `guild` decoder) sitting at a literal byte offset inside it,
//! alongside what looks like the base name as a length-prefixed UTF-16
//! string.
//!
//! The map value is a struct of five properties — `WorkerDirector`,
//! `WorkCollection`, `ModuleMap`, `RawData`, `CustomVersionData` — the first
//! two of which carry a *nested* `RawData` blob of their own. Which Pals work
//! here is not in the base's own `RawData`, as one might expect: it is a
//! container id inside the sibling `WorkerDirector.RawData`.
//!
//! ## `WorkerDirector.RawData`, a fixed 118 bytes
//!
//! | Offset | Bytes | Meaning |
//! |---|---|---|
//! | 0 | 16 | the base's own id — identical to the map key, so it is a free parse check |
//! | 16 | 80 | an `FTransform`: quat (4×f64), translation (3×f64), scale (3×f64) |
//! | 96 | 2 | `00 01` when the base has workers, `00 00` when it does not |
//! | 98 | 16 | the worker container id |
//! | 114 | 4 | zero |
//!
//! Verified across multiple decoded bases: the id at offset 0 matched the map
//! key, and the container at offset 98 accounted for the unowned Pals without
//! omissions or duplicate claims. Bases without workers still carry an empty
//! container id rather than omitting the field, so a missing id means "did not
//! decode", never "no workers". `tests/real_base_camps.rs` pins both claims.
//!
//! Two further fields decode cleanly but are deliberately unused: the base's
//! own `RawData` holds a UTF-16 name at offset 16, which is the placeholder
//! `新規生成拠点テンプレート名N(仮)` for every base (Palworld gives no way to
//! name one), and a `u32` at offset 56 that reads `1` everywhere and so
//! distinguishes nothing. Fully reverse-engineering the rest — `ModuleMap`'s
//! buildings, `WorkCollection`'s work orders, HP, level — is real future work.

use uuid::Uuid;

use crate::gvas_ext::{find, struct_properties};
use paldex_gvas::Value;

/// `WorkerDirector.RawData`'s exact length. A blob of any other size is a
/// layout we have not seen and must not read offsets out of.
const WORKER_DIRECTOR_LEN: usize = 118;

/// Where the worker container id starts within `WorkerDirector.RawData`.
const WORKER_CONTAINER_OFFSET: usize = 98;

#[derive(Debug, Clone, serde::Serialize)]
pub struct BaseCamp {
    pub id: Uuid,
    /// `None` if no id in `known_guild_ids` was found inside this base's raw
    /// bytes — could mean an unowned base, or just that the byte-scan missed
    /// it (this isn't a verified-exact field offset, see module docs).
    pub guild_id: Option<Uuid>,
    /// The container every Pal working at this base sits in, joined against
    /// `Pal::location`'s `container_id`.
    ///
    /// `None` only if `WorkerDirector` was missing, the wrong length, or did
    /// not begin with this base's own id — an *empty* base still has a
    /// container id here, so `None` means "could not decode", never "no
    /// workers".
    pub worker_container_id: Option<Uuid>,
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

    let guild_id = raw_data(props, "RawData")
        .and_then(|bytes| find_embedded_guid(bytes, known_guild_ids));
    let worker_container_id =
        raw_data(props, "WorkerDirector").and_then(|bytes| worker_container(bytes, id));

    Some(BaseCamp {
        id,
        guild_id,
        worker_container_id,
    })
}

/// The `RawData` blob for `name`, whether it *is* that property (the base's
/// own) or sits one level down inside it (`WorkerDirector`'s).
fn raw_data<'a>(props: &'a [paldex_gvas::Property], name: &str) -> Option<&'a [u8]> {
    match find(props, name)? {
        Value::Raw(bytes) => Some(bytes),
        nested => match find(struct_properties(nested)?, "RawData")? {
            Value::Raw(bytes) => Some(bytes),
            _ => None,
        },
    }
}

/// The container id at [`WORKER_CONTAINER_OFFSET`], but only out of a blob
/// whose length and leading self-id both check out — a layout that has moved
/// must yield `None`, never a plausible-looking `Uuid` sliced from the wrong
/// field.
fn worker_container(bytes: &[u8], base_id: Uuid) -> Option<Uuid> {
    if bytes.len() != WORKER_DIRECTOR_LEN || bytes[..16] != *base_id.as_bytes() {
        return None;
    }
    let mut guid = [0u8; 16];
    guid.copy_from_slice(&bytes[WORKER_CONTAINER_OFFSET..WORKER_CONTAINER_OFFSET + 16]);
    Some(Uuid::from_bytes(guid))
}

fn find_embedded_guid(haystack: &[u8], candidates: &[Uuid]) -> Option<Uuid> {
    candidates
        .iter()
        .find(|id| haystack.windows(16).any(|w| w == id.as_bytes()))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(base_id: Uuid, container: Uuid) -> Vec<u8> {
        let mut bytes = vec![0u8; WORKER_DIRECTOR_LEN];
        bytes[..16].copy_from_slice(base_id.as_bytes());
        bytes[WORKER_CONTAINER_OFFSET..WORKER_CONTAINER_OFFSET + 16]
            .copy_from_slice(container.as_bytes());
        bytes
    }

    #[test]
    fn a_well_formed_blob_yields_its_container() {
        let base = Uuid::from_u128(1);
        let container = Uuid::from_u128(2);
        assert_eq!(worker_container(&blob(base, container), base), Some(container));
    }

    #[test]
    fn a_blob_of_the_wrong_length_yields_none() {
        let base = Uuid::from_u128(1);
        let mut short = blob(base, Uuid::from_u128(2));
        short.pop();
        assert_eq!(worker_container(&short, base), None);
    }

    #[test]
    fn a_blob_whose_self_id_disagrees_with_the_map_key_yields_none() {
        let bytes = blob(Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(worker_container(&bytes, Uuid::from_u128(99)), None);
    }
}
