//! The tagged-property list format that follows the [`crate::header::Header`].
//!
//! Every property is a self-describing tag:
//!
//! ```text
//! name: FString              ("None" terminates the list)
//! type_name: FString
//! size: i64                  byte length of `value`, below
//! [type-specific extra]      see `TypeExtra`
//! has_property_guid: bool
//! [property_guid: 16 bytes]  present iff has_property_guid
//! value: `size` bytes
//! ```
//!
//! `size` is the load-bearing field for robustness: it always covers exactly the
//! `value` bytes (verified against `IntProperty`, `StrProperty`, `ArrayProperty`,
//! `MapProperty`, and struct fields on a real save), so after decoding a value —
//! confidently or not — the reader can jump straight to `value_start + size` for
//! the next tag. A property type or nested struct this reader doesn't specifically
//! understand degrades to [`Value::Raw`] instead of corrupting everything after it.

use serde::Serialize;

use crate::cursor::Cursor;
use crate::GvasError;

/// Struct names Unreal serializes as compact fixed-width binary rather than a
/// nested tagged-property list. Verified against real data: `Guid` (a
/// `PlayerUId`/`InstanceId` field) and `DateTime` (`LevelMeta.sav`'s `Timestamp`).
/// Anything else is assumed to be one of Palworld's own custom structs, which use
/// the tagged-property-list encoding — verified against `PalIndividualCharacterSaveParameter`
/// and the top-level `PalWorldSaveData` struct itself.
const COMPACT_GUID: &str = "Guid";
const COMPACT_DATETIME: &str = "DateTime";
const COMPACT_TIMESPAN: &str = "Timespan";

/// Upper bound on a single property's declared value length. Real values top out
/// around 7.5 MB (`CharacterSaveParameterMap`); this leaves headroom while still
/// rejecting a clearly corrupt length before any allocation.
const MAX_VALUE_LEN: i64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Property {
    pub name: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Value {
    Int(i32),
    Int64(i64),
    UInt32(u32),
    Float(f32),
    Double(f64),
    Bool(bool),
    Str(String),
    Name(String),
    Enum {
        enum_type: String,
        value: String,
    },
    Byte {
        enum_type: Option<String>,
        value: ByteValue,
    },
    Struct {
        struct_name: String,
        value: StructValue,
    },
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Set(Vec<Value>),
    /// Bytes this reader doesn't decode further: `ArrayProperty<ByteProperty>`
    /// (Palworld's `RawData` fields — nested guild/inventory/base-camp blobs
    /// left for Phase 2's dedicated decoders), and the safe fallback for any
    /// property type or struct this reader doesn't specifically understand.
    Raw(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ByteValue {
    Raw(u8),
    Enum(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum StructValue {
    Guid([u8; 16]),
    /// Ticks since the .NET epoch, for both `DateTime` and `Timespan` structs.
    DateTime(i64),
    Properties(Vec<Property>),
    Raw(Vec<u8>),
}

/// The type-specific header fields that sit between a tag's `size` and its
/// `has_property_guid` byte.
enum TypeExtra {
    None,
    Struct { struct_name: String },
    Enum { enum_name: String },
    Byte { enum_name: String },
    Array { inner_type: String },
    Set { inner_type: String },
    Map { key_type: String, value_type: String },
}

impl TypeExtra {
    fn read(cursor: &mut Cursor, type_name: &str) -> Result<Self, GvasError> {
        Ok(match type_name {
            "StructProperty" => {
                let struct_name = cursor.fstring()?;
                cursor.guid()?; // struct_guid: always zero on every real property observed
                TypeExtra::Struct { struct_name }
            }
            "EnumProperty" => TypeExtra::Enum {
                enum_name: cursor.fstring()?,
            },
            "ByteProperty" => TypeExtra::Byte {
                enum_name: cursor.fstring()?,
            },
            "ArrayProperty" => TypeExtra::Array {
                inner_type: cursor.fstring()?,
            },
            "SetProperty" => TypeExtra::Set {
                inner_type: cursor.fstring()?,
            },
            "MapProperty" => TypeExtra::Map {
                key_type: cursor.fstring()?,
                value_type: cursor.fstring()?,
            },
            _ => TypeExtra::None,
        })
    }
}

/// Parse a standalone buffer as a `None`-terminated property list.
///
/// Meant for re-parsing an already-extracted [`Value::Raw`] blob — Palworld's
/// `RawData` fields hold exactly this shape (see the module docs) — without
/// requiring a full `GVAS` header.
pub fn parse_property_list_bytes(bytes: &[u8]) -> Result<Vec<Property>, GvasError> {
    let mut cursor = Cursor::new(bytes);
    parse_property_list(&mut cursor)
}

/// Parse a `None`-terminated list of property tags — the shape of the GVAS body
/// right after the header, and of every generic (non-compact) struct's contents.
pub(crate) fn parse_property_list(cursor: &mut Cursor) -> Result<Vec<Property>, GvasError> {
    let mut props = Vec::new();
    while let Some(prop) = parse_property(cursor)? {
        props.push(prop);
    }
    Ok(props)
}

fn parse_property(cursor: &mut Cursor) -> Result<Option<Property>, GvasError> {
    let name = cursor.fstring()?;
    if name == "None" {
        return Ok(None);
    }
    let type_name = cursor.fstring()?;
    let size = cursor.i64()?;
    if !(0..=MAX_VALUE_LEN).contains(&size) {
        return Err(GvasError::ImplausibleLength {
            declared: size,
            property: name,
        });
    }
    #[allow(clippy::cast_sign_loss)]
    let size = size as usize;

    // BoolProperty is the one type whose "value" is inline before HasPropertyGuid,
    // with Size always 0 — handle it before the generic extra/guid/value dance.
    if type_name == "BoolProperty" {
        let b = cursor.bool()?;
        let has_guid = cursor.bool()?;
        if has_guid {
            cursor.guid()?;
        }
        return Ok(Some(Property {
            name,
            value: Value::Bool(b),
        }));
    }

    let extra = TypeExtra::read(cursor, &type_name)?;

    let has_guid = cursor.bool()?;
    if has_guid {
        cursor.guid()?;
    }

    let value_end = cursor.pos() + size;
    let value = decode_value(cursor, &type_name, &extra, size)?;
    // Unconditional resync: whatever decode_value actually consumed, the next
    // tag starts exactly `size` bytes after the value began.
    cursor.set_pos(value_end)?;

    Ok(Some(Property { name, value }))
}

fn decode_value(
    cursor: &mut Cursor,
    type_name: &str,
    extra: &TypeExtra,
    size: usize,
) -> Result<Value, GvasError> {
    match type_name {
        "IntProperty" => Ok(Value::Int(cursor.i32()?)),
        "Int64Property" => Ok(Value::Int64(cursor.i64()?)),
        "UInt32Property" => Ok(Value::UInt32(cursor.u32()?)),
        "FloatProperty" => Ok(Value::Float(cursor.f32()?)),
        "DoubleProperty" => Ok(Value::Double(cursor.f64()?)),
        "StrProperty" => Ok(Value::Str(cursor.fstring()?)),
        "NameProperty" => Ok(Value::Name(cursor.fstring()?)),
        "EnumProperty" => {
            let TypeExtra::Enum { enum_name } = extra else {
                unreachable!("EnumProperty always carries TypeExtra::Enum")
            };
            Ok(Value::Enum {
                enum_type: enum_name.clone(),
                value: cursor.fstring()?,
            })
        }
        "ByteProperty" => {
            let TypeExtra::Byte { enum_name } = extra else {
                unreachable!("ByteProperty always carries TypeExtra::Byte")
            };
            if enum_name == "None" {
                Ok(Value::Byte {
                    enum_type: None,
                    value: ByteValue::Raw(cursor.u8()?),
                })
            } else {
                Ok(Value::Byte {
                    enum_type: Some(enum_name.clone()),
                    value: ByteValue::Enum(cursor.fstring()?),
                })
            }
        }
        "StructProperty" => {
            let TypeExtra::Struct { struct_name } = extra else {
                unreachable!("StructProperty always carries TypeExtra::Struct")
            };
            Ok(Value::Struct {
                struct_name: struct_name.clone(),
                value: decode_struct(cursor, struct_name, size)?,
            })
        }
        "ArrayProperty" => {
            let TypeExtra::Array { inner_type } = extra else {
                unreachable!("ArrayProperty always carries TypeExtra::Array")
            };
            decode_array(cursor, inner_type, size)
        }
        "SetProperty" => {
            let TypeExtra::Set { inner_type } = extra else {
                unreachable!("SetProperty always carries TypeExtra::Set")
            };
            decode_set(cursor, inner_type, size)
        }
        "MapProperty" => {
            let TypeExtra::Map {
                key_type,
                value_type,
            } = extra
            else {
                unreachable!("MapProperty always carries TypeExtra::Map")
            };
            decode_map(cursor, key_type, value_type, size)
        }
        _ => Ok(Value::Raw(cursor.sub_cursor(size)?.all().to_vec())),
    }
}

fn decode_struct(cursor: &mut Cursor, struct_name: &str, size: usize) -> Result<StructValue, GvasError> {
    match struct_name {
        COMPACT_GUID => Ok(StructValue::Guid(cursor.guid()?)),
        COMPACT_DATETIME | COMPACT_TIMESPAN => Ok(StructValue::DateTime(cursor.i64()?)),
        _ => {
            let mut sub = cursor.sub_cursor(size)?;
            match parse_property_list(&mut sub) {
                Ok(props) => Ok(StructValue::Properties(props)),
                Err(_) => Ok(StructValue::Raw(cursor.sub_cursor(size)?.all().to_vec())),
            }
        }
    }
}

/// A single map/set key or value of a scalar or struct type. Palworld's own
/// structs (verified: `CharacterSaveParameterMap`'s `StructProperty` keys) are
/// read as a plain tagged-property list, the same as a generic struct field —
/// there is no per-element struct name to resolve compact-vs-list here, since
/// none of the real collections carry compact-struct (`Guid`/`DateTime`) elements.
fn decode_collection_element(cursor: &mut Cursor, type_name: &str) -> Result<Value, GvasError> {
    match type_name {
        "StructProperty" => Ok(Value::Struct {
            struct_name: String::new(),
            value: StructValue::Properties(parse_property_list(cursor)?),
        }),
        "IntProperty" => Ok(Value::Int(cursor.i32()?)),
        "Int64Property" => Ok(Value::Int64(cursor.i64()?)),
        "UInt32Property" => Ok(Value::UInt32(cursor.u32()?)),
        "FloatProperty" => Ok(Value::Float(cursor.f32()?)),
        "DoubleProperty" => Ok(Value::Double(cursor.f64()?)),
        "BoolProperty" => Ok(Value::Bool(cursor.bool()?)),
        "StrProperty" => Ok(Value::Str(cursor.fstring()?)),
        "NameProperty" => Ok(Value::Name(cursor.fstring()?)),
        "ByteProperty" => Ok(Value::Byte {
            enum_type: None,
            value: ByteValue::Raw(cursor.u8()?),
        }),
        "EnumProperty" => Ok(Value::Enum {
            enum_type: String::new(),
            value: cursor.fstring()?,
        }),
        other => Err(GvasError::UnsupportedCollectionElement(other.to_owned())),
    }
}

fn decode_array(cursor: &mut Cursor, inner_type: &str, size: usize) -> Result<Value, GvasError> {
    if inner_type == "ByteProperty" {
        // RawData and friends: keep the payload, drop the redundant count prefix.
        let mut sub = cursor.sub_cursor(size)?;
        let _count = sub.u32()?;
        return Ok(Value::Raw(sub.bytes(sub.len() - sub.pos())?.to_vec()));
    }

    match try_decode_array(cursor, inner_type, size) {
        Ok(v) => Ok(v),
        Err(_) => Ok(Value::Raw(cursor.sub_cursor(size)?.all().to_vec())),
    }
}

fn try_decode_array(cursor: &mut Cursor, inner_type: &str, size: usize) -> Result<Value, GvasError> {
    let mut sub = cursor.sub_cursor(size)?;
    let count = sub.u32()? as usize;
    let mut items = Vec::with_capacity(count.min(4096));

    if inner_type == "StructProperty" {
        // Arrays of structs carry one leading "dummy" tag declaring the struct
        // type shared by every element, before the elements themselves.
        let _dummy_name = sub.fstring()?;
        let dummy_type = sub.fstring()?;
        let dummy_size = sub.i64()?;
        if dummy_type != "StructProperty" || dummy_size < 0 {
            return Err(GvasError::UnsupportedCollectionElement(inner_type.to_owned()));
        }
        let struct_name = sub.fstring()?;
        sub.guid()?;
        // The dummy tag is a full property tag in miniature, right down to its
        // own has_property_guid byte — verified against a real
        // CharacterContainerSaveData/PalCharacterSlotSaveData array; missing
        // this one byte silently shifted every element read by one byte.
        if sub.bool()? {
            sub.guid()?;
        }
        for _ in 0..count {
            let props = parse_property_list(&mut sub)?;
            items.push(Value::Struct {
                struct_name: struct_name.clone(),
                value: StructValue::Properties(props),
            });
        }
        return Ok(Value::Array(items));
    }

    for _ in 0..count {
        items.push(decode_collection_element(&mut sub, inner_type)?);
    }
    Ok(Value::Array(items))
}

fn decode_set(cursor: &mut Cursor, inner_type: &str, size: usize) -> Result<Value, GvasError> {
    match try_decode_set(cursor, inner_type, size) {
        Ok(v) => Ok(v),
        Err(_) => Ok(Value::Raw(cursor.sub_cursor(size)?.all().to_vec())),
    }
}

fn try_decode_set(cursor: &mut Cursor, inner_type: &str, size: usize) -> Result<Value, GvasError> {
    let mut sub = cursor.sub_cursor(size)?;
    let _removed = sub.u32()?;
    let count = sub.u32()? as usize;
    let mut items = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        items.push(decode_collection_element(&mut sub, inner_type)?);
    }
    Ok(Value::Set(items))
}

fn decode_map(
    cursor: &mut Cursor,
    key_type: &str,
    value_type: &str,
    size: usize,
) -> Result<Value, GvasError> {
    match try_decode_map(cursor, key_type, value_type, size) {
        Ok(v) => Ok(v),
        Err(_) => Ok(Value::Raw(cursor.sub_cursor(size)?.all().to_vec())),
    }
}

fn try_decode_map(
    cursor: &mut Cursor,
    key_type: &str,
    value_type: &str,
    size: usize,
) -> Result<Value, GvasError> {
    let mut sub = cursor.sub_cursor(size)?;
    let _unused = sub.u32()?;
    let count = sub.u32()? as usize;
    let mut entries = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        let key = decode_collection_element(&mut sub, key_type)?;
        let value = decode_collection_element(&mut sub, value_type)?;
        entries.push((key, value));
    }
    Ok(Value::Map(entries))
}
