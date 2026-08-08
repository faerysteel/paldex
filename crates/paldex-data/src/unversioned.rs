//! Deserializer for Unreal's *unversioned* property serialization.
//!
//! Cooked packages with `PKG_UnversionedProperties` drop property names and
//! types from the payload entirely. What remains is a compact header saying
//! *which* schema slots are present, followed by their raw values in schema
//! order. Recovering a value therefore needs the schema from a `.usmap`
//! ([`crate::usmap`]) and nothing else.
//!
//! ## The header
//!
//! A run-length list of fragments, each a little-endian `u16`:
//!
//! ```text
//! bits 0..6   SkipNum        schema slots to skip before this run
//! bit  7      bHasAnyZeroes  this run's values are zero-mask eligible
//! bit  8      bIsLast        final fragment
//! bits 9..15  ValueNum       values in this run
//! ```
//!
//! Fragments are read until `bIsLast`. If any fragment set `bHasAnyZeroes`, a
//! bitmask follows with one bit per value in those fragments — a set bit means
//! the value is zero and occupies **no bytes at all**. The mask is stored as a
//! `u8` for up to 8 bits, a `u16` for up to 16, and 32-bit words beyond that.
//!
//! The bit layout is not guesswork: the alternative packing (`ValueNum` at
//! bit 8, `bIsLast` at bit 15) fails to terminate on the real
//! `DT_PalMonsterParameter`, while this one reproduces its property section
//! exactly and lands precisely where the row data begins.
//!
//! ## Values
//!
//! Values are raw and untagged, so every size must be exactly right or the
//! rest of the stream desynchronizes — which is also what makes this cheap to
//! verify: a correct decoder consumes an export's bytes to the *exact* byte.

use std::collections::HashMap;

use crate::uasset::{Reader, UassetError};
use crate::usmap::{PropertyType, Usmap};

/// Structs the engine serializes with hand-written code rather than through
/// their property schema. Reading them via the schema would look for an
/// unversioned header that was never written.
///
/// UE5 stores core geometry types as doubles (large-world coordinates), so
/// `FVector` is 24 bytes, not 12.
fn native_struct_size(name: &str) -> Option<usize> {
    Some(match name {
        "Vector" | "Rotator" => 24,
        "Vector2D" => 16,
        "Vector4" | "Quat" | "Plane" => 32,
        "IntPoint" => 8,
        "IntVector" => 12,
        "Guid" => 16,
        "Color" => 4,
        "LinearColor" => 16,
        "Box" => 49,
        "Box2D" => 33,
        _ => return None,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum UnversionedError {
    #[error("no schema in the .usmap for struct {0:?}")]
    UnknownStruct(String),
    #[error("payload references schema slot {index} of {struct_name}, which the .usmap does not describe")]
    MissingSchemaSlot { struct_name: String, index: usize },
    #[error("property type {0} is not supported by this reader")]
    UnsupportedType(&'static str),
    #[error("struct {0:?} has no schema and no known native layout")]
    UnknownNativeStruct(String),
    #[error(transparent)]
    Read(#[from] UassetError),
}

/// A decoded property value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    /// An `FName`, resolved against the package's name table.
    Name(String),
    Str(String),
    /// An enum entry's name, or its raw integer if the `.usmap` has no entry.
    Enum(String),
    /// An `FPackageIndex`: negative selects an import, positive an export.
    Object(i32),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Struct(Properties),
    /// A value present in the payload whose bytes were skipped because the
    /// caller does not need it (native structs, delegates, text).
    Opaque,
    /// A value the zero-mask elided. Semantically the type's default: 0,
    /// `false`, or an empty string/array.
    Zero,
}

impl Value {
    /// This value as an integer, treating an elided zero as 0.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Bool(b) => Some(i64::from(*b)),
            Value::Zero => Some(0),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_i32(&self) -> Option<i32> {
        self.as_i64().and_then(|v| i32::try_from(v).ok())
    }

    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            Value::Int(v) => Some(*v as f64),
            Value::Zero => Some(0.0),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Int(v) => Some(*v != 0),
            Value::Zero => Some(false),
            _ => None,
        }
    }

    /// This value as a string, treating an elided zero as empty.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Name(s) | Value::Enum(s) => Some(s),
            Value::Zero => Some(""),
            _ => None,
        }
    }
}

/// The decoded properties of one struct, keyed by property name.
///
/// Properties the payload omitted entirely are absent rather than `Zero`:
/// absence means "the writer had nothing to say", which for a `DataTable` row
/// means the field holds its default.
pub type Properties = HashMap<String, Value>;

/// One fragment of the unversioned header.
struct Fragment {
    skip: u32,
    has_zeroes: bool,
    is_last: bool,
    values: u32,
}

impl Fragment {
    fn unpack(packed: u16) -> Self {
        let packed = u32::from(packed);
        Self {
            skip: packed & 0x7F,
            has_zeroes: packed & 0x80 != 0,
            is_last: packed & 0x100 != 0,
            values: packed >> 9,
        }
    }
}

/// Reads unversioned property payloads against a `.usmap` schema.
pub struct UnversionedReader<'a> {
    usmap: &'a Usmap,
    names: &'a [String],
}

impl<'a> UnversionedReader<'a> {
    /// `names` is the owning package's name table, used to resolve `FName`s.
    #[must_use]
    pub fn new(usmap: &'a Usmap, names: &'a [String]) -> Self {
        Self { usmap, names }
    }

    /// Read one struct's unversioned properties at the reader's position.
    pub fn read_struct(
        &self,
        r: &mut Reader<'_>,
        struct_name: &str,
    ) -> Result<Properties, UnversionedError> {
        let schema = self.usmap.flat_schema(struct_name);
        if schema.is_empty() && self.usmap.get_struct(struct_name).is_none() {
            return Err(UnversionedError::UnknownStruct(struct_name.to_owned()));
        }

        let mut fragments = Vec::new();
        let mut zero_bits = 0u32;
        loop {
            let fragment = Fragment::unpack(r.u16()?);
            if fragment.has_zeroes {
                zero_bits += fragment.values;
            }
            let last = fragment.is_last;
            fragments.push(fragment);
            if last {
                break;
            }
        }

        let zero_mask = read_zero_mask(r, zero_bits)?;

        let mut out = Properties::new();
        let mut slot = 0usize;
        let mut zero_index = 0usize;
        for fragment in &fragments {
            slot += fragment.skip as usize;
            for _ in 0..fragment.values {
                let is_zero = if fragment.has_zeroes {
                    let bit = zero_mask.get(zero_index).copied().unwrap_or(false);
                    zero_index += 1;
                    bit
                } else {
                    false
                };

                let property = schema.get(slot).copied().flatten().ok_or_else(|| {
                    UnversionedError::MissingSchemaSlot {
                        struct_name: struct_name.to_owned(),
                        index: slot,
                    }
                })?;

                let value = if is_zero {
                    Value::Zero
                } else if property.array_size > 1 {
                    // A C-style fixed array occupies one schema slot but
                    // serializes every element back to back.
                    let mut items = Vec::with_capacity(property.array_size as usize);
                    for _ in 0..property.array_size {
                        items.push(self.read_value(r, &property.ty)?);
                    }
                    Value::Array(items)
                } else {
                    self.read_value(r, &property.ty)?
                };
                out.insert(property.name.clone(), value);
                slot += 1;
            }
        }

        Ok(out)
    }

    fn read_value(
        &self,
        r: &mut Reader<'_>,
        ty: &PropertyType,
    ) -> Result<Value, UnversionedError> {
        Ok(match ty {
            PropertyType::Bool => Value::Bool(r.u8()? != 0),
            PropertyType::Byte | PropertyType::Int8 => Value::Int(i64::from(r.u8()?)),
            PropertyType::Int16 | PropertyType::UInt16 => Value::Int(i64::from(r.u16()?)),
            PropertyType::Int | PropertyType::UInt32 => Value::Int(i64::from(r.i32()?)),
            PropertyType::Int64 | PropertyType::UInt64 => Value::Int(r.i64()?),
            PropertyType::Float => Value::Float(f64::from(r.f32()?)),
            PropertyType::Double => Value::Float(r.f64()?),
            PropertyType::Name => Value::Name(self.read_name(r)?),
            PropertyType::Str | PropertyType::Utf8Str | PropertyType::AnsiStr => {
                Value::Str(r.fstring()?)
            }
            PropertyType::Object
            | PropertyType::WeakObject
            | PropertyType::LazyObject
            | PropertyType::SoftObject
            | PropertyType::AssetObject
            | PropertyType::Interface => Value::Object(r.i32()?),
            PropertyType::Enum { inner, enum_name } => {
                let raw = self.read_value(r, inner)?;
                let index = raw.as_i64().and_then(|v| usize::try_from(v).ok());
                match index.and_then(|i| self.usmap.enum_value(enum_name, i)) {
                    Some(entry) => Value::Enum(entry.to_owned()),
                    None => raw,
                }
            }
            PropertyType::Array(inner) | PropertyType::Set(inner) => {
                let count = r.i32()?.max(0) as usize;
                let mut items = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    items.push(self.read_value(r, inner)?);
                }
                Value::Array(items)
            }
            PropertyType::Map { key, value } => {
                let count = r.i32()?.max(0) as usize;
                let mut items = Vec::with_capacity(count.min(4096));
                for _ in 0..count {
                    let k = self.read_value(r, key)?;
                    let v = self.read_value(r, value)?;
                    items.push((k, v));
                }
                Value::Map(items)
            }
            PropertyType::Struct(name) => {
                if let Some(size) = native_struct_size(name) {
                    r.skip(size)?;
                    Value::Opaque
                } else if self.usmap.get_struct(name).is_some() {
                    Value::Struct(self.read_struct(r, name)?)
                } else {
                    return Err(UnversionedError::UnknownNativeStruct(name.clone()));
                }
            }
            PropertyType::Optional(inner) => {
                if r.u8()? == 0 {
                    Value::Zero
                } else {
                    self.read_value(r, inner)?
                }
            }
            // These carry variable-length payloads whose layout this reader
            // does not model. Nothing the tracker needs uses them, and
            // guessing a size would silently corrupt everything after.
            PropertyType::Text => return Err(UnversionedError::UnsupportedType("Text")),
            PropertyType::Delegate => return Err(UnversionedError::UnsupportedType("Delegate")),
            PropertyType::MulticastDelegate => {
                return Err(UnversionedError::UnsupportedType("MulticastDelegate"))
            }
            PropertyType::FieldPath => return Err(UnversionedError::UnsupportedType("FieldPath")),
        })
    }

    fn read_name(&self, r: &mut Reader<'_>) -> Result<String, UnversionedError> {
        let index = r.i32()?;
        let _number = r.i32()?;
        Ok(usize::try_from(index)
            .ok()
            .and_then(|i| self.names.get(i))
            .cloned()
            .unwrap_or_default())
    }
}

/// Read the zero-mask bits that follow the fragment list.
fn read_zero_mask(r: &mut Reader<'_>, bits: u32) -> Result<Vec<bool>, UassetError> {
    if bits == 0 {
        return Ok(Vec::new());
    }
    let mut mask = Vec::with_capacity(bits as usize);
    if bits <= 8 {
        let word = r.u8()?;
        for i in 0..bits {
            mask.push(word >> i & 1 == 1);
        }
    } else if bits <= 16 {
        let word = r.u16()?;
        for i in 0..bits {
            mask.push(word >> i & 1 == 1);
        }
    } else {
        let words = bits.div_ceil(32);
        for w in 0..words {
            let word = r.u32()?;
            for i in 0..32 {
                if w * 32 + i < bits {
                    mask.push(word >> i & 1 == 1);
                }
            }
        }
    }
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Row { Value: Int (0), Kind: Enum<EKind> (1), Label: Str (2) }`
    fn usmap() -> Usmap {
        let names = ["Row", "Value", "Kind", "Label", "EKind", "A", "B"];
        let mut body = Vec::new();
        body.extend_from_slice(&(names.len() as i32).to_le_bytes());
        for n in names {
            body.push(n.len() as u8);
            body.extend_from_slice(n.as_bytes());
        }
        body.extend_from_slice(&1i32.to_le_bytes()); // one enum
        body.extend_from_slice(&4i32.to_le_bytes()); // EKind
        body.extend_from_slice(&2i32.to_le_bytes());
        body.extend_from_slice(&5i32.to_le_bytes()); // A
        body.extend_from_slice(&6i32.to_le_bytes()); // B

        body.extend_from_slice(&1i32.to_le_bytes()); // one struct
        body.extend_from_slice(&0i32.to_le_bytes()); // "Row"
        body.extend_from_slice(&(-1i32).to_le_bytes());
        body.extend_from_slice(&3u16.to_le_bytes()); // property_count
        body.extend_from_slice(&3u16.to_le_bytes()); // serializable
        for (index, name_idx, tag) in [(0u16, 1i32, 2u8), (1, 2, 26), (2, 3, 10)] {
            body.extend_from_slice(&index.to_le_bytes());
            body.push(1);
            body.extend_from_slice(&name_idx.to_le_bytes());
            body.push(tag);
            if tag == 26 {
                body.push(0); // inner: Byte
                body.extend_from_slice(&4i32.to_le_bytes()); // EKind
            }
        }

        let mut file = Vec::new();
        file.extend_from_slice(&0x30C4u16.to_le_bytes());
        file.push(0);
        file.push(0);
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);
        Usmap::parse(&file).expect("usmap")
    }

    fn fragment(skip: u32, has_zeroes: bool, is_last: bool, values: u32) -> u16 {
        let mut packed = skip & 0x7F;
        if has_zeroes {
            packed |= 0x80;
        }
        if is_last {
            packed |= 0x100;
        }
        packed |= values << 9;
        packed as u16
    }

    #[test]
    fn reads_consecutive_values() {
        let m = usmap();
        let names: Vec<String> = vec![];
        let reader = UnversionedReader::new(&m, &names);

        let mut buf = Vec::new();
        buf.extend_from_slice(&fragment(0, false, true, 3).to_le_bytes());
        buf.extend_from_slice(&7i32.to_le_bytes()); // Value
        buf.push(1); // Kind -> B
        buf.extend_from_slice(&3i32.to_le_bytes()); // Label "Hi\0"
        buf.extend_from_slice(b"Hi\0");

        let mut r = Reader::new(&buf);
        let props = reader.read_struct(&mut r, "Row").expect("read");
        assert_eq!(props["Value"], Value::Int(7));
        assert_eq!(props["Kind"], Value::Enum("B".into()));
        assert_eq!(props["Label"], Value::Str("Hi".into()));
        assert_eq!(r.pos(), buf.len(), "must consume the payload exactly");
    }

    /// A skip must advance the schema cursor without consuming bytes.
    #[test]
    fn skipped_slots_are_absent_not_defaulted() {
        let m = usmap();
        let names: Vec<String> = vec![];
        let reader = UnversionedReader::new(&m, &names);

        let mut buf = Vec::new();
        buf.extend_from_slice(&fragment(2, false, true, 1).to_le_bytes());
        buf.extend_from_slice(&2i32.to_le_bytes());
        buf.extend_from_slice(b"X\0");

        let mut r = Reader::new(&buf);
        let props = reader.read_struct(&mut r, "Row").expect("read");
        assert_eq!(props["Label"], Value::Str("X".into()));
        assert!(!props.contains_key("Value"));
        assert!(!props.contains_key("Kind"));
        assert_eq!(r.pos(), buf.len());
    }

    /// A zero-masked value occupies no bytes at all — the property that
    /// follows must still land correctly.
    #[test]
    fn zero_masked_values_consume_no_bytes() {
        let m = usmap();
        let names: Vec<String> = vec![];
        let reader = UnversionedReader::new(&m, &names);

        let mut buf = Vec::new();
        buf.extend_from_slice(&fragment(0, true, true, 2).to_le_bytes());
        buf.push(0b01); // Value is zero, Kind is not
        buf.push(1); // Kind -> B

        let mut r = Reader::new(&buf);
        let props = reader.read_struct(&mut r, "Row").expect("read");
        assert_eq!(props["Value"], Value::Zero);
        assert_eq!(props["Value"].as_i64(), Some(0));
        assert_eq!(props["Kind"], Value::Enum("B".into()));
        assert_eq!(r.pos(), buf.len());
    }

    #[test]
    fn multiple_fragments_accumulate() {
        let m = usmap();
        let names: Vec<String> = vec![];
        let reader = UnversionedReader::new(&m, &names);

        let mut buf = Vec::new();
        buf.extend_from_slice(&fragment(0, false, false, 1).to_le_bytes());
        buf.extend_from_slice(&fragment(1, false, true, 1).to_le_bytes());
        buf.extend_from_slice(&9i32.to_le_bytes()); // Value
        buf.extend_from_slice(&2i32.to_le_bytes()); // Label
        buf.extend_from_slice(b"Z\0");

        let mut r = Reader::new(&buf);
        let props = reader.read_struct(&mut r, "Row").expect("read");
        assert_eq!(props["Value"], Value::Int(9));
        assert_eq!(props["Label"], Value::Str("Z".into()));
        assert!(!props.contains_key("Kind"));
        assert_eq!(r.pos(), buf.len());
    }

    /// An enum value with no matching entry must degrade to its raw integer
    /// rather than being dropped or guessed at.
    #[test]
    fn out_of_range_enum_falls_back_to_its_integer() {
        let m = usmap();
        let names: Vec<String> = vec![];
        let reader = UnversionedReader::new(&m, &names);

        let mut buf = Vec::new();
        buf.extend_from_slice(&fragment(1, false, true, 1).to_le_bytes());
        buf.push(200);

        let mut r = Reader::new(&buf);
        let props = reader.read_struct(&mut r, "Row").expect("read");
        assert_eq!(props["Kind"], Value::Int(200));
    }

    #[test]
    fn unknown_struct_is_an_error() {
        let m = usmap();
        let names: Vec<String> = vec![];
        let reader = UnversionedReader::new(&m, &names);
        let buf = [0u8; 8];
        let mut r = Reader::new(&buf);
        assert!(matches!(
            reader.read_struct(&mut r, "Nope"),
            Err(UnversionedError::UnknownStruct(_))
        ));
    }

    #[test]
    fn truncation_never_panics() {
        let m = usmap();
        let names: Vec<String> = vec![];
        let reader = UnversionedReader::new(&m, &names);

        let mut buf = Vec::new();
        buf.extend_from_slice(&fragment(0, false, true, 3).to_le_bytes());
        buf.extend_from_slice(&7i32.to_le_bytes());
        buf.push(1);
        buf.extend_from_slice(&3i32.to_le_bytes());
        buf.extend_from_slice(b"Hi\0");

        for n in 0..buf.len() {
            let mut r = Reader::new(&buf[..n]);
            let _ = reader.read_struct(&mut r, "Row");
        }
    }

    #[test]
    fn zero_mask_widths_round_trip() {
        for bits in [1u32, 8, 9, 16, 17, 33, 64] {
            let bytes = match bits {
                0..=8 => 1,
                9..=16 => 2,
                n => (n.div_ceil(32) * 4) as usize,
            };
            let buf = vec![0xFF; bytes];
            let mut r = Reader::new(&buf);
            let mask = read_zero_mask(&mut r, bits).expect("mask");
            assert_eq!(mask.len(), bits as usize);
            assert!(mask.iter().all(|b| *b));
            assert_eq!(r.pos(), bytes);
        }
    }
}
