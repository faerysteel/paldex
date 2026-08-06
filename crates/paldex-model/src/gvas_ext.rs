//! Small typed accessors over `paldex_gvas::Property` slices, shared by every
//! `rawdata` decoder. Each just narrows a `Value` to the shape a known field is
//! verified to have; a field that's missing or a different shape than expected
//! yields `None`/empty rather than panicking, consistent with `paldex-gvas`'s
//! own "degrade, don't desync" philosophy.

use std::collections::{HashMap, HashSet};

use paldex_gvas::{ByteValue, Property, StructValue, Value};
use uuid::Uuid;

pub fn find<'a>(props: &'a [Property], name: &str) -> Option<&'a Value> {
    props.iter().find(|p| p.name == name).map(|p| &p.value)
}

pub fn struct_properties(value: &Value) -> Option<&[Property]> {
    match value {
        Value::Struct {
            value: StructValue::Properties(props),
            ..
        } => Some(props),
        _ => None,
    }
}

pub fn find_struct_properties<'a>(props: &'a [Property], name: &str) -> Option<&'a [Property]> {
    struct_properties(find(props, name)?)
}

pub fn find_guid(props: &[Property], name: &str) -> Option<Uuid> {
    match find(props, name) {
        Some(Value::Struct {
            value: StructValue::Guid(bytes),
            ..
        }) => Some(Uuid::from_bytes(*bytes)),
        _ => None,
    }
}

/// A `PalContainerId`-shaped struct: `{ ID: Guid }`.
pub fn find_container_id(props: &[Property], name: &str) -> Option<Uuid> {
    find_guid(find_struct_properties(props, name)?, "ID")
}

pub fn find_byte(props: &[Property], name: &str) -> Option<u8> {
    match find(props, name) {
        Some(Value::Byte {
            value: ByteValue::Raw(b),
            ..
        }) => Some(*b),
        _ => None,
    }
}

pub fn find_int(props: &[Property], name: &str) -> Option<i32> {
    match find(props, name) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

pub fn find_str<'a>(props: &'a [Property], name: &str) -> Option<&'a str> {
    match find(props, name) {
        Some(Value::Str(s)) => Some(s.as_str()),
        _ => None,
    }
}

pub fn find_enum<'a>(props: &'a [Property], name: &str) -> Option<&'a str> {
    match find(props, name) {
        Some(Value::Enum { value, .. }) => Some(value.as_str()),
        _ => None,
    }
}

pub fn find_name_array(props: &[Property], name: &str) -> Vec<String> {
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

pub fn find_enum_array(props: &[Property], name: &str) -> Vec<String> {
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

/// A `Map<Name, Bool>` field, as the set of keys whose value is `true`.
pub fn find_map_true_keys(props: &[Property], name: &str) -> HashSet<String> {
    match find(props, name) {
        Some(Value::Map(entries)) => entries
            .iter()
            .filter_map(|(k, v)| match (k, v) {
                (Value::Name(s), Value::Bool(true)) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => HashSet::new(),
    }
}

/// A `Map<Name, Int>` field.
pub fn find_map_int(props: &[Property], name: &str) -> HashMap<String, u32> {
    match find(props, name) {
        Some(Value::Map(entries)) => entries
            .iter()
            .filter_map(|(k, v)| match (k, v) {
                (Value::Name(s), Value::Int(n)) => Some((s.clone(), u32::try_from(*n).unwrap_or(0))),
                _ => None,
            })
            .collect(),
        _ => HashMap::new(),
    }
}

pub fn u32_or_zero(n: Option<i32>) -> u32 {
    n.and_then(|v| u32::try_from(v).ok()).unwrap_or(0)
}
