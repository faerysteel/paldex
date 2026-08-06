//! Minimal reader for cooked Unreal `.uasset` package headers.
//!
//! Only what Phase 3 actually needs: the package flags (to know whether
//! property values require a `.usmap` schema) and the **name table**, which is
//! stored as plain `FString`s regardless of whether the package uses
//! unversioned property serialization. That distinction is the whole reason
//! this module is useful — see [`crate::reference`] for the full writeup.

/// `EPackageFlags::PKG_UnversionedProperties` — when set, property *values* in
/// this package are serialized without names/types and need a `.usmap` schema.
pub const PKG_UNVERSIONED_PROPERTIES: u32 = 0x0000_2000;

const PACKAGE_FILE_TAG: u32 = 0x9E2A_83C1;

/// Refuse to allocate for an `FString` longer than this. Real name-table
/// entries are short identifiers; anything larger means we have desynced from
/// the real structure and should error rather than allocate wildly.
const MAX_FSTRING_LEN: i32 = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum UassetError {
    #[error("not a cooked uasset package: expected tag {PACKAGE_FILE_TAG:#010x}, found {found:#010x}")]
    BadTag { found: u32 },
    #[error("unexpected end of package data at offset {offset}")]
    Truncated { offset: usize },
    #[error("implausible string length {len} at offset {offset}")]
    BadStringLength { len: i32, offset: usize },
    #[error("name table offset {offset} out of range (package is {len} bytes)")]
    BadNameTable { offset: usize, len: usize },
}

/// The parts of `FPackageFileSummary` this crate uses.
#[derive(Debug, Clone)]
pub struct PackageSummary {
    pub package_flags: u32,
    /// Package path as cooked, e.g. `/Game/Pal/DataTable/Character/DT_…`.
    pub folder_name: String,
    /// The package's name table, in index order.
    pub names: Vec<String>,
}

impl PackageSummary {
    /// Whether property values in this package need a `.usmap` to deserialize.
    #[must_use]
    pub fn has_unversioned_properties(&self) -> bool {
        self.package_flags & PKG_UNVERSIONED_PROPERTIES != 0
    }
}

/// A bounds-checked little-endian byte reader. Every accessor returns `Result`
/// so that truncated or corrupt package data surfaces as an error rather than
/// a panic — the plan makes "no panics on truncated input" an explicit gate.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(crate) fn at(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], UassetError> {
        let end = self.pos.checked_add(n).ok_or(UassetError::Truncated { offset: self.pos })?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(UassetError::Truncated { offset: self.pos })?;
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn u32(&mut self) -> Result<u32, UassetError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn i32(&mut self) -> Result<i32, UassetError> {
        Ok(self.u32()? as i32)
    }

    pub(crate) fn i64(&mut self) -> Result<i64, UassetError> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes(b.try_into().expect("took exactly 8 bytes")))
    }

    pub(crate) fn skip(&mut self, n: usize) -> Result<(), UassetError> {
        self.take(n).map(|_| ())
    }

    /// Unreal `FString`: length-prefixed, NUL-terminated. A positive length is
    /// a byte count of ASCII/UTF-8; a negative length is a UTF-16 *character*
    /// count.
    pub(crate) fn fstring(&mut self) -> Result<String, UassetError> {
        let offset = self.pos;
        let len = self.i32()?;
        if len == 0 {
            return Ok(String::new());
        }
        if len.unsigned_abs() > MAX_FSTRING_LEN.unsigned_abs() {
            return Err(UassetError::BadStringLength { len, offset });
        }
        if len > 0 {
            let raw = self.take(len as usize)?;
            // Trim the trailing NUL terminator.
            let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
            Ok(String::from_utf8_lossy(raw).into_owned())
        } else {
            let count = len.unsigned_abs() as usize;
            let raw = self.take(count * 2)?;
            let mut units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            if units.last() == Some(&0) {
                units.pop();
            }
            Ok(String::from_utf16_lossy(&units))
        }
    }
}

/// Parse a cooked `.uasset` package header and its name table.
pub fn parse_summary(bytes: &[u8]) -> Result<PackageSummary, UassetError> {
    let mut r = Reader::new(bytes);
    let tag = r.u32()?;
    if tag != PACKAGE_FILE_TAG {
        return Err(UassetError::BadTag { found: tag });
    }

    let legacy_file_version = r.i32()?;
    if legacy_file_version != -4 {
        r.skip(4)?; // LegacyUE3Version
    }
    r.skip(4)?; // FileVersionUE4
    if legacy_file_version <= -8 {
        r.skip(4)?; // FileVersionUE5
    }
    r.skip(4)?; // FileVersionLicenseeUE4

    let custom_version_count = r.i32()?;
    if custom_version_count < 0 {
        return Err(UassetError::Truncated { offset: r.pos() });
    }
    // Each custom version is a 16-byte GUID plus an i32 version.
    r.skip((custom_version_count as usize).saturating_mul(20))?;

    r.skip(4)?; // TotalHeaderSize
    let folder_name = r.fstring()?;
    let package_flags = r.u32()?;
    let name_count = r.i32()?;
    let name_offset = r.i32()?;

    let mut names = Vec::new();
    if name_count > 0 && name_offset > 0 {
        let offset = name_offset as usize;
        if offset >= bytes.len() {
            return Err(UassetError::BadNameTable { offset, len: bytes.len() });
        }
        let mut n = Reader::at(bytes, offset);
        names.reserve(name_count as usize);
        for _ in 0..name_count {
            names.push(n.fstring()?);
            // Each entry carries precomputed case-sensitive/insensitive hashes.
            n.skip(4)?;
        }
    }

    Ok(PackageSummary { package_flags, folder_name, names })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii_fstring(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let len = s.len() as i32 + 1;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        out
    }

    /// Build a minimal but structurally real cooked package header.
    fn synthetic_package(flags: u32, names: &[&str]) -> Vec<u8> {
        let mut header = Vec::new();
        header.extend_from_slice(&PACKAGE_FILE_TAG.to_le_bytes());
        header.extend_from_slice(&(-8i32).to_le_bytes()); // legacy file version
        header.extend_from_slice(&0i32.to_le_bytes()); // legacy UE3
        header.extend_from_slice(&0i32.to_le_bytes()); // FileVersionUE4
        header.extend_from_slice(&0i32.to_le_bytes()); // FileVersionUE5
        header.extend_from_slice(&0i32.to_le_bytes()); // licensee
        header.extend_from_slice(&0i32.to_le_bytes()); // custom version count
        header.extend_from_slice(&0i32.to_le_bytes()); // total header size
        header.extend_from_slice(&ascii_fstring("/Game/Test"));
        header.extend_from_slice(&flags.to_le_bytes());
        header.extend_from_slice(&(names.len() as i32).to_le_bytes());
        let name_offset_pos = header.len();
        header.extend_from_slice(&0i32.to_le_bytes()); // placeholder

        let name_offset = header.len() as i32;
        header[name_offset_pos..name_offset_pos + 4].copy_from_slice(&name_offset.to_le_bytes());
        for n in names {
            header.extend_from_slice(&ascii_fstring(n));
            header.extend_from_slice(&0u32.to_le_bytes()); // hashes
        }
        header
    }

    #[test]
    fn parses_flags_and_name_table() {
        let bytes = synthetic_package(PKG_UNVERSIONED_PROPERTIES, &["Alpaca", "BOSS_Alpaca"]);
        let s = parse_summary(&bytes).expect("parse");
        assert!(s.has_unversioned_properties());
        assert_eq!(s.folder_name, "/Game/Test");
        assert_eq!(s.names, vec!["Alpaca", "BOSS_Alpaca"]);
    }

    #[test]
    fn detects_tagged_properties() {
        let bytes = synthetic_package(0, &["Alpaca"]);
        let s = parse_summary(&bytes).expect("parse");
        assert!(!s.has_unversioned_properties());
    }

    #[test]
    fn rejects_non_uasset_input() {
        let err = parse_summary(&[0, 1, 2, 3, 4, 5, 6, 7]).unwrap_err();
        assert!(matches!(err, UassetError::BadTag { .. }));
    }

    /// Truncating at every length must error, never panic — a corrupt or
    /// partially-read pak entry has to stay recoverable.
    #[test]
    fn truncation_never_panics() {
        let full = synthetic_package(PKG_UNVERSIONED_PROPERTIES, &["Alpaca", "Anubis"]);
        for n in 0..full.len() {
            let _ = parse_summary(&full[..n]);
        }
    }

    #[test]
    fn rejects_absurd_string_length() {
        let mut bytes = synthetic_package(0, &[]);
        // Point the name table at a bogus, enormous length.
        bytes.extend_from_slice(&i32::MAX.to_le_bytes());
        let mut r = Reader::at(&bytes, bytes.len() - 4);
        assert!(matches!(r.fstring(), Err(UassetError::BadStringLength { .. })));
    }
}
