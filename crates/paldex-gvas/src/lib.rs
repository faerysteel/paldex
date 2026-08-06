//! A reader for Unreal Engine's `GVAS` SaveGame binary format, as written by
//! Palworld on UE 5.1.1.
//!
//! Two layers:
//! - [`header::Header`] — the fixed-shape preamble (engine version, custom format
//!   versions, save-game class name), verified byte-for-byte against a real save.
//! - [`value`] — the tagged-property list that follows it, recursively covering
//!   scalars, structs, arrays, maps, and sets.
//!
//! Every property tag declares its own value's byte length, which this reader
//! leans on heavily for robustness: a struct, array, or map whose internal shape
//! isn't fully understood degrades to [`Value::Raw`] rather than corrupting the
//! properties that follow it. `RawData` — Palworld's own nested guild/inventory/
//! base-camp blobs — is *always* opaque here by design; decoding it is Phase 2's
//! job.

mod cursor;
mod header;
mod value;

pub use header::{CustomVersion, EngineVersion, Header};
pub use value::{parse_property_list_bytes, ByteValue, Property, StructValue, Value};

use cursor::Cursor;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum GvasError {
    #[error("input does not start with the GVAS magic")]
    NotGvas,

    #[error("unexpected end of input at offset {offset} (needed {needed} more bytes, {available} available)")]
    UnexpectedEof {
        offset: usize,
        needed: usize,
        available: usize,
    },

    #[error("string length {0} would require negating i32::MIN, which is not representable")]
    InvalidStringLength(i32),

    #[error("property {property:?} declares an implausible value length {declared}")]
    ImplausibleLength { declared: i64, property: String },

    #[error("{what} declares an implausible count {declared} (limit {limit})")]
    ImplausibleCount {
        declared: u32,
        limit: u32,
        what: &'static str,
    },

    #[error("unsupported collection element type {0:?}")]
    UnsupportedCollectionElement(String),
}

/// A parsed `GVAS` file: its header plus the top-level property list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Root {
    pub header: Header,
    pub properties: Vec<Property>,
}

impl Root {
    /// The value of a top-level property by name, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.properties
            .iter()
            .find(|p| p.name == name)
            .map(|p| &p.value)
    }
}

/// Parse a decompressed `GVAS` payload — the output of `paldex_sav::decompress` —
/// into a header and a property tree.
pub fn parse(gvas_bytes: &[u8]) -> Result<Root, GvasError> {
    let mut cursor = Cursor::new(gvas_bytes);
    let header = Header::parse(&mut cursor)?;
    let properties = value::parse_property_list(&mut cursor)?;
    Ok(Root { header, properties })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_bytes() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"GVAS");
        b.extend_from_slice(&3u32.to_le_bytes()); // save_game_version
        b.extend_from_slice(&522u32.to_le_bytes()); // package_file_version_ue4
        b.extend_from_slice(&1008u32.to_le_bytes()); // package_file_version_ue5
        b.extend_from_slice(&5u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // changelist
        push_fstring(&mut b, "++UE5+Release-5.1");
        b.extend_from_slice(&3u32.to_le_bytes()); // custom_format_version
        b.extend_from_slice(&0u32.to_le_bytes()); // 0 custom versions, for a minimal fixture
        push_fstring(&mut b, "/Script/Pal.PalWorldBaseInfoSaveGame");
        b
    }

    fn push_fstring(b: &mut Vec<u8>, s: &str) {
        let bytes = s.as_bytes();
        b.extend_from_slice(&(bytes.len() as i32 + 1).to_le_bytes());
        b.extend_from_slice(bytes);
        b.push(0);
    }

    fn push_int_property(b: &mut Vec<u8>, name: &str, value: i32) {
        push_fstring(b, name);
        push_fstring(b, "IntProperty");
        b.extend_from_slice(&4i64.to_le_bytes());
        b.push(0); // has_property_guid
        b.extend_from_slice(&value.to_le_bytes());
    }

    fn push_none(b: &mut Vec<u8>) {
        push_fstring(b, "None");
    }

    #[test]
    fn rejects_non_gvas_input() {
        let err = parse(b"not a gvas file").unwrap_err();
        assert!(matches!(err, GvasError::NotGvas));
    }

    #[test]
    fn parses_the_verified_header_shape() {
        let mut b = header_bytes();
        push_none(&mut b);

        let root = parse(&b).unwrap();
        assert_eq!(root.header.save_game_version, 3);
        assert_eq!(root.header.package_file_version_ue4, 522);
        assert_eq!(root.header.package_file_version_ue5, 1008);
        assert_eq!(root.header.engine_version.major, 5);
        assert_eq!(root.header.engine_version.minor, 1);
        assert_eq!(root.header.engine_version.patch, 1);
        assert_eq!(root.header.engine_version.branch, "++UE5+Release-5.1");
        assert_eq!(root.header.custom_format_version, 3);
        assert_eq!(
            root.header.save_game_class_name,
            "/Script/Pal.PalWorldBaseInfoSaveGame"
        );
        assert!(root.properties.is_empty());
    }

    #[test]
    fn parses_a_flat_int_property() {
        let mut b = header_bytes();
        push_int_property(&mut b, "Version", 100);
        push_none(&mut b);

        let root = parse(&b).unwrap();
        assert_eq!(root.get("Version"), Some(&Value::Int(100)));
    }

    #[test]
    fn parses_bool_true_and_false() {
        let mut b = header_bytes();
        push_fstring(&mut b, "bFlagTrue");
        push_fstring(&mut b, "BoolProperty");
        b.extend_from_slice(&0i64.to_le_bytes());
        b.push(1); // the bool value itself
        b.push(0); // has_property_guid
        push_fstring(&mut b, "bFlagFalse");
        push_fstring(&mut b, "BoolProperty");
        b.extend_from_slice(&0i64.to_le_bytes());
        b.push(0);
        b.push(0);
        push_none(&mut b);

        let root = parse(&b).unwrap();
        assert_eq!(root.get("bFlagTrue"), Some(&Value::Bool(true)));
        assert_eq!(root.get("bFlagFalse"), Some(&Value::Bool(false)));
    }

    #[test]
    fn parses_a_compact_guid_struct() {
        let mut b = header_bytes();
        push_fstring(&mut b, "InstanceId");
        push_fstring(&mut b, "StructProperty");
        b.extend_from_slice(&16i64.to_le_bytes());
        push_fstring(&mut b, "Guid");
        b.extend_from_slice(&[0u8; 16]); // struct_guid
        b.push(0); // has_property_guid
        let guid = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        b.extend_from_slice(&guid);
        push_none(&mut b);

        let root = parse(&b).unwrap();
        assert_eq!(
            root.get("InstanceId"),
            Some(&Value::Struct {
                struct_name: "Guid".to_owned(),
                value: StructValue::Guid(guid),
            })
        );
    }

    #[test]
    fn parses_a_generic_struct_as_a_nested_property_list() {
        let mut inner = Vec::new();
        push_int_property(&mut inner, "Level", 42);
        push_none(&mut inner);

        let mut b = header_bytes();
        push_fstring(&mut b, "SaveParameter");
        push_fstring(&mut b, "StructProperty");
        b.extend_from_slice(&(inner.len() as i64).to_le_bytes());
        push_fstring(&mut b, "PalIndividualCharacterSaveParameter");
        b.extend_from_slice(&[0u8; 16]);
        b.push(0);
        b.extend_from_slice(&inner);
        push_none(&mut b);

        let root = parse(&b).unwrap();
        let Some(Value::Struct { struct_name, value }) = root.get("SaveParameter") else {
            panic!("expected a struct");
        };
        assert_eq!(struct_name, "PalIndividualCharacterSaveParameter");
        let StructValue::Properties(props) = value else {
            panic!("expected a nested property list, got {value:?}");
        };
        assert_eq!(props[0].name, "Level");
        assert_eq!(props[0].value, Value::Int(42));
    }

    #[test]
    fn parses_raw_data_as_an_array_of_byte_property() {
        let mut b = header_bytes();
        push_fstring(&mut b, "RawData");
        push_fstring(&mut b, "ArrayProperty");
        let payload = [0xAAu8, 0xBB, 0xCC];
        b.extend_from_slice(&(4 + payload.len() as i64).to_le_bytes());
        push_fstring(&mut b, "ByteProperty");
        b.push(0);
        b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        b.extend_from_slice(&payload);
        push_none(&mut b);

        let root = parse(&b).unwrap();
        assert_eq!(root.get("RawData"), Some(&Value::Raw(payload.to_vec())));
    }

    #[test]
    fn parses_a_map_of_struct_keys_and_int_values() {
        let mut key = Vec::new();
        push_int_property(&mut key, "InstanceId", 7);
        push_none(&mut key);

        let mut b = header_bytes();
        push_fstring(&mut b, "Counts");
        push_fstring(&mut b, "MapProperty");
        let map_value_len = 4 + 4 + key.len() + 4; // unused + count + one entry (key + int value)
        b.extend_from_slice(&(map_value_len as i64).to_le_bytes());
        push_fstring(&mut b, "StructProperty");
        push_fstring(&mut b, "IntProperty");
        b.push(0);
        b.extend_from_slice(&0u32.to_le_bytes()); // unused
        b.extend_from_slice(&1u32.to_le_bytes()); // count
        b.extend_from_slice(&key);
        b.extend_from_slice(&9i32.to_le_bytes());
        push_none(&mut b);

        let root = parse(&b).unwrap();
        let Some(Value::Map(entries)) = root.get("Counts") else {
            panic!("expected a map");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, Value::Int(9));
    }

    #[test]
    fn an_unknown_property_type_degrades_to_raw_without_desyncing() {
        let mut b = header_bytes();
        push_fstring(&mut b, "Mystery");
        push_fstring(&mut b, "SomeFutureProperty");
        b.extend_from_slice(&4i64.to_le_bytes());
        b.push(0);
        b.extend_from_slice(&[1, 2, 3, 4]);
        push_int_property(&mut b, "AfterMystery", 55);
        push_none(&mut b);

        let root = parse(&b).unwrap();
        assert_eq!(root.get("Mystery"), Some(&Value::Raw(vec![1, 2, 3, 4])));
        assert_eq!(root.get("AfterMystery"), Some(&Value::Int(55)));
    }

    #[test]
    fn a_malformed_generic_struct_degrades_to_raw_without_desyncing() {
        let mut b = header_bytes();
        push_fstring(&mut b, "Weird");
        push_fstring(&mut b, "StructProperty");
        let garbage = [0xFFu8; 8];
        b.extend_from_slice(&(garbage.len() as i64).to_le_bytes());
        push_fstring(&mut b, "SomeUnknownCompactType");
        b.extend_from_slice(&[0u8; 16]);
        b.push(0);
        b.extend_from_slice(&garbage);
        push_int_property(&mut b, "AfterWeird", 7);
        push_none(&mut b);

        let root = parse(&b).unwrap();
        assert!(matches!(
            root.get("Weird"),
            Some(Value::Struct {
                value: StructValue::Raw(_) | StructValue::Properties(_),
                ..
            })
        ));
        assert_eq!(root.get("AfterWeird"), Some(&Value::Int(7)));
    }

    #[test]
    fn truncated_input_errors_rather_than_panics() {
        let mut b = header_bytes();
        push_int_property(&mut b, "Version", 100);
        b.truncate(b.len() - 10);

        assert!(parse(&b).is_err());
    }
}
