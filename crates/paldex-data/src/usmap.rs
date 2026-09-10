//! Reader for Unreal `.usmap` mappings files.
//!
//! Supplies the positional property schema omitted by unversioned cooked
//! packages, including the rows decoded by [`crate::datatable`].
//!
//! Generate one with `tools/usmap/regen-usmap.ps1`; see
//! `tools/usmap/README.md` for the patched dumper workflow.
//!
//! ## Format
//!
//! ```text
//! u16 magic (0x30C4) | u8 version | u8 compression | u32 compressed | u32 decompressed
//! u32 nameCount   ; each: u8 length + UTF-8 bytes
//! u32 enumCount   ; each: i32 nameIdx, i32 entryCount, entryCount * i32 nameIdx
//! u32 structCount ; each: i32 nameIdx, i32 superIdx (-1 = none),
//!                         u16 propCount, u16 serializablePropCount,
//!                         each: u16 schemaIdx, u8 arraySize, i32 nameIdx, PropertyType
//! ```
//!
//! Supports version 0 (`Initial`) with compression method 0 (uncompressed).
//! Other versions and compression methods are rejected.

use std::collections::HashMap;

use crate::uasset::{Reader, UassetError};

const USMAP_MAGIC: u16 = 0x30C4;

/// `EUsmapVersion::Initial`. Later versions change the name and enum encodings.
const SUPPORTED_VERSION: u8 = 0;

#[derive(Debug, thiserror::Error)]
pub enum UsmapError {
    #[error("not a .usmap file: expected magic {USMAP_MAGIC:#06x}, found {found:#06x}")]
    BadMagic { found: u16 },
    #[error("unsupported .usmap version {0} (only version {SUPPORTED_VERSION} is supported)")]
    UnsupportedVersion(u8),
    #[error("unsupported .usmap compression method {0} (only uncompressed is supported)")]
    Compressed(u8),
    #[error("body is {actual} bytes but the header declares {declared}")]
    SizeMismatch { declared: u32, actual: usize },
    #[error("name index {index} out of range (the file has {count} names)")]
    BadNameIndex { index: i32, count: usize },
    #[error("unknown property type {0}")]
    UnknownPropertyType(u8),
    #[error("negative count {0}")]
    NegativeCount(i32),
    #[error(transparent)]
    Read(#[from] UassetError),
}

/// A property's type, as the schema describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyType {
    Byte,
    Bool,
    Int,
    Float,
    Object,
    Name,
    Delegate,
    Double,
    Array(Box<PropertyType>),
    Struct(String),
    Str,
    Text,
    Interface,
    MulticastDelegate,
    WeakObject,
    LazyObject,
    AssetObject,
    SoftObject,
    UInt64,
    UInt32,
    UInt16,
    Int64,
    Int16,
    Int8,
    Map { key: Box<PropertyType>, value: Box<PropertyType> },
    Set(Box<PropertyType>),
    /// An enum stored as `inner` (usually a byte), named by `enum_name`.
    Enum { inner: Box<PropertyType>, enum_name: String },
    FieldPath,
    Optional(Box<PropertyType>),
    Utf8Str,
    AnsiStr,
}

/// One serializable property in a struct's schema.
#[derive(Debug, Clone)]
pub struct Property {
    /// Position in the owning struct's schema, which is how unversioned data
    /// refers to it.
    pub index: u16,
    /// Number of elements for a C-style fixed array (`Thing[4]`); 1 normally.
    pub array_size: u8,
    pub name: String,
    pub ty: PropertyType,
}

/// A struct or class layout.
#[derive(Debug, Clone)]
pub struct Struct {
    pub name: String,
    /// Parent struct/class, whose properties follow this one's in the
    /// flattened schema — see [`Usmap::flat_schema`].
    pub super_name: Option<String>,
    /// Total schema slots, including any this file has no entry for.
    pub property_count: u16,
    pub properties: Vec<Property>,
}

/// A parsed `.usmap`.
#[derive(Debug, Clone, Default)]
pub struct Usmap {
    names: Vec<String>,
    enums: HashMap<String, Vec<String>>,
    structs: HashMap<String, Struct>,
}

impl Usmap {
    /// Parse a `.usmap` file's bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, UsmapError> {
        let mut r = Reader::new(bytes);
        let magic = r.u16()?;
        if magic != USMAP_MAGIC {
            return Err(UsmapError::BadMagic { found: magic });
        }
        let version = r.u8()?;
        if version != SUPPORTED_VERSION {
            return Err(UsmapError::UnsupportedVersion(version));
        }
        let compression = r.u8()?;
        if compression != 0 {
            return Err(UsmapError::Compressed(compression));
        }
        let _compressed_size = r.u32()?;
        let decompressed_size = r.u32()?;
        let body_len = bytes.len() - r.pos();
        if body_len != decompressed_size as usize {
            return Err(UsmapError::SizeMismatch {
                declared: decompressed_size,
                actual: body_len,
            });
        }

        let name_count = count(r.i32()?)?;
        let mut names = Vec::with_capacity(name_count);
        for _ in 0..name_count {
            let len = r.u8()? as usize;
            names.push(String::from_utf8_lossy(r.take(len)?).into_owned());
        }

        let enum_count = count(r.i32()?)?;
        let mut enums = HashMap::with_capacity(enum_count);
        for _ in 0..enum_count {
            let name = name_at(&names, r.i32()?)?;
            let entry_count = count(r.i32()?)?;
            let mut entries = Vec::with_capacity(entry_count.min(1024));
            for _ in 0..entry_count {
                entries.push(name_at(&names, r.i32()?)?);
            }
            enums.insert(name, entries);
        }

        let struct_count = count(r.i32()?)?;
        let mut structs = HashMap::with_capacity(struct_count);
        for _ in 0..struct_count {
            let name = name_at(&names, r.i32()?)?;
            let super_index = r.i32()?;
            let super_name =
                if super_index < 0 { None } else { Some(name_at(&names, super_index)?) };
            let property_count = r.u16()?;
            let serializable = r.u16()?;
            let mut properties = Vec::with_capacity(serializable as usize);
            for _ in 0..serializable {
                let index = r.u16()?;
                let array_size = r.u8()?;
                let prop_name = name_at(&names, r.i32()?)?;
                let ty = parse_type(&mut r, &names)?;
                properties.push(Property { index, array_size, name: prop_name, ty });
            }
            structs.insert(
                name.clone(),
                Struct { name, super_name, property_count, properties },
            );
        }

        Ok(Self { names, enums, structs })
    }

    #[must_use]
    pub fn name_count(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub fn enum_count(&self) -> usize {
        self.enums.len()
    }

    #[must_use]
    pub fn struct_count(&self) -> usize {
        self.structs.len()
    }

    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    #[must_use]
    pub fn get_struct(&self, name: &str) -> Option<&Struct> {
        self.structs.get(name)
    }

    /// Resolve an enum entry by its stored integer value.
    #[must_use]
    pub fn enum_value(&self, enum_name: &str, value: usize) -> Option<&str> {
        self.enums.get(enum_name)?.get(value).map(String::as_str)
    }

    #[must_use]
    pub fn enum_entries(&self, enum_name: &str) -> Option<&[String]> {
        self.enums.get(enum_name).map(Vec::as_slice)
    }

    /// The schema an unversioned payload indexes into: this struct's own
    /// properties first, then its super's, and so on up the chain.
    ///
    /// **The derived-first order is load-bearing and was established
    /// empirically**, not assumed. Reading the real
    /// `DT_PalMonsterParameter` export with `CompositeDataTable` this way
    /// yields `ParentTables = [import -3]` (the `_Common` table it composes)
    /// and `RowStruct = [import -7]` (`PalCharacterParameterDatabaseRow`) —
    /// both semantically exact, and the properties end precisely where the
    /// row data begins. Super-first ordering instead reads `RowStruct` as
    /// export index 1 (the table itself) and desynchronizes immediately.
    ///
    /// Slots the file lists no property for come back as `None` so that
    /// indices still line up; a payload that actually references one is an
    /// error for the caller to report, not something to silently shift past.
    #[must_use]
    pub fn flat_schema(&self, struct_name: &str) -> Vec<Option<&Property>> {
        let mut out = Vec::new();
        let mut current = Some(struct_name.to_owned());
        // Guard against a malformed file whose super chain loops.
        let mut seen = std::collections::HashSet::new();
        while let Some(name) = current {
            if !seen.insert(name.clone()) {
                break;
            }
            let Some(s) = self.structs.get(&name) else {
                break;
            };
            for index in 0..s.property_count {
                out.push(s.properties.iter().find(|p| p.index == index));
            }
            current = s.super_name.clone();
        }
        out
    }
}

fn count(n: i32) -> Result<usize, UsmapError> {
    usize::try_from(n).map_err(|_| UsmapError::NegativeCount(n))
}

fn name_at(names: &[String], index: i32) -> Result<String, UsmapError> {
    usize::try_from(index)
        .ok()
        .and_then(|i| names.get(i))
        .cloned()
        .ok_or(UsmapError::BadNameIndex { index, count: names.len() })
}

fn parse_type(r: &mut Reader, names: &[String]) -> Result<PropertyType, UsmapError> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => PropertyType::Byte,
        1 => PropertyType::Bool,
        2 => PropertyType::Int,
        3 => PropertyType::Float,
        4 => PropertyType::Object,
        5 => PropertyType::Name,
        6 => PropertyType::Delegate,
        7 => PropertyType::Double,
        8 => PropertyType::Array(Box::new(parse_type(r, names)?)),
        9 => PropertyType::Struct(name_at(names, r.i32()?)?),
        10 => PropertyType::Str,
        11 => PropertyType::Text,
        12 => PropertyType::Interface,
        13 => PropertyType::MulticastDelegate,
        14 => PropertyType::WeakObject,
        15 => PropertyType::LazyObject,
        16 => PropertyType::AssetObject,
        17 => PropertyType::SoftObject,
        18 => PropertyType::UInt64,
        19 => PropertyType::UInt32,
        20 => PropertyType::UInt16,
        21 => PropertyType::Int64,
        22 => PropertyType::Int16,
        23 => PropertyType::Int8,
        24 => {
            let key = Box::new(parse_type(r, names)?);
            let value = Box::new(parse_type(r, names)?);
            PropertyType::Map { key, value }
        }
        25 => PropertyType::Set(Box::new(parse_type(r, names)?)),
        26 => {
            let inner = Box::new(parse_type(r, names)?);
            let enum_name = name_at(names, r.i32()?)?;
            PropertyType::Enum { inner, enum_name }
        }
        27 => PropertyType::FieldPath,
        28 => PropertyType::Optional(Box::new(parse_type(r, names)?)),
        29 => PropertyType::Utf8Str,
        30 => PropertyType::AnsiStr,
        other => return Err(UsmapError::UnknownPropertyType(other)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal but structurally real `.usmap`.
    fn synthetic() -> Vec<u8> {
        let names = ["Row", "Base", "Value", "Kind", "EKind", "A", "B", "Extra"];
        let mut body = Vec::new();
        body.extend_from_slice(&(names.len() as i32).to_le_bytes());
        for n in names {
            body.push(n.len() as u8);
            body.extend_from_slice(n.as_bytes());
        }
        // one enum: EKind { A, B }
        body.extend_from_slice(&1i32.to_le_bytes());
        body.extend_from_slice(&4i32.to_le_bytes()); // name "EKind"
        body.extend_from_slice(&2i32.to_le_bytes()); // 2 entries
        body.extend_from_slice(&5i32.to_le_bytes()); // "A"
        body.extend_from_slice(&6i32.to_le_bytes()); // "B"
        // two structs: Base { Extra: Int }, Row : Base { Value: Int, Kind: Enum<EKind> }
        body.extend_from_slice(&2i32.to_le_bytes());

        body.extend_from_slice(&1i32.to_le_bytes()); // "Base"
        body.extend_from_slice(&(-1i32).to_le_bytes()); // no super
        body.extend_from_slice(&1u16.to_le_bytes()); // property_count
        body.extend_from_slice(&1u16.to_le_bytes()); // serializable
        body.extend_from_slice(&0u16.to_le_bytes());
        body.push(1);
        body.extend_from_slice(&7i32.to_le_bytes()); // "Extra"
        body.push(2); // Int

        body.extend_from_slice(&0i32.to_le_bytes()); // "Row"
        body.extend_from_slice(&1i32.to_le_bytes()); // super "Base"
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.push(1);
        body.extend_from_slice(&2i32.to_le_bytes()); // "Value"
        body.push(2); // Int
        body.extend_from_slice(&1u16.to_le_bytes());
        body.push(1);
        body.extend_from_slice(&3i32.to_le_bytes()); // "Kind"
        body.push(26); // Enum
        body.push(0); // inner: Byte
        body.extend_from_slice(&4i32.to_le_bytes()); // "EKind"

        let mut out = Vec::new();
        out.extend_from_slice(&USMAP_MAGIC.to_le_bytes());
        out.push(0); // version
        out.push(0); // compression
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_names_enums_and_structs() {
        let m = Usmap::parse(&synthetic()).expect("parse");
        assert_eq!(m.name_count(), 8);
        assert_eq!(m.enum_count(), 1);
        assert_eq!(m.struct_count(), 2);
        assert_eq!(m.enum_value("EKind", 1), Some("B"));
        assert_eq!(m.enum_value("EKind", 9), None);
        assert!(m.contains_name("Value"));
    }

    /// The derived-first order is what the real package data requires; a
    /// regression here silently desynchronizes every unversioned read.
    #[test]
    fn flat_schema_puts_derived_properties_before_super() {
        let m = Usmap::parse(&synthetic()).expect("parse");
        let schema = m.flat_schema("Row");
        let names: Vec<_> = schema.iter().map(|p| p.map(|p| p.name.as_str())).collect();
        assert_eq!(names, vec![Some("Value"), Some("Kind"), Some("Extra")]);
    }

    #[test]
    fn enum_property_carries_its_enum_name() {
        let m = Usmap::parse(&synthetic()).expect("parse");
        let row = m.get_struct("Row").expect("Row");
        let kind = row.properties.iter().find(|p| p.name == "Kind").expect("Kind");
        assert_eq!(
            kind.ty,
            PropertyType::Enum {
                inner: Box::new(PropertyType::Byte),
                enum_name: "EKind".into()
            }
        );
    }

    #[test]
    fn rejects_foreign_or_unsupported_files() {
        assert!(matches!(
            Usmap::parse(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
            Err(UsmapError::BadMagic { .. })
        ));

        let mut v = synthetic();
        v[2] = 1; // version 1
        assert!(matches!(Usmap::parse(&v), Err(UsmapError::UnsupportedVersion(1))));

        let mut c = synthetic();
        c[3] = 1; // compression: Oodle
        assert!(matches!(Usmap::parse(&c), Err(UsmapError::Compressed(1))));
    }

    /// A truncated file must error rather than panic — the same gate the rest
    /// of the crate holds itself to.
    #[test]
    fn truncation_never_panics() {
        let full = synthetic();
        for n in 0..full.len() {
            let _ = Usmap::parse(&full[..n]);
        }
    }

    /// A super chain that loops must terminate rather than hang.
    #[test]
    fn flat_schema_survives_a_looping_super_chain() {
        let mut m = Usmap::default();
        m.structs.insert(
            "A".into(),
            Struct {
                name: "A".into(),
                super_name: Some("B".into()),
                property_count: 0,
                properties: vec![],
            },
        );
        m.structs.insert(
            "B".into(),
            Struct {
                name: "B".into(),
                super_name: Some("A".into()),
                property_count: 0,
                properties: vec![],
            },
        );
        assert!(m.flat_schema("A").is_empty());
    }
}
