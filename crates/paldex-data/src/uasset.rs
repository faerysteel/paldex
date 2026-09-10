//! Minimal reader for cooked Unreal `.uasset` package headers.
//!
//! Reads package flags, name tables, and import/export metadata.
//! Name tables remain readable without an unversioned-property schema.

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
    #[error("name index {index} out of range (the package has {count} names)")]
    BadNameIndex { index: i32, count: usize },
    #[error(
        "export map is {span} bytes for {count} exports, not the expected \
         {expected} bytes each — the cooked package layout has changed"
    )]
    ExportMapStride { span: usize, count: usize, expected: usize },
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

/// Little-endian byte reader. Reads are bounds-checked and return
/// [`UassetError::Truncated`] on insufficient input.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub fn at(bytes: &'a [u8], pos: usize) -> Self {
        Self { bytes, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], UassetError> {
        let end = self.pos.checked_add(n).ok_or(UassetError::Truncated { offset: self.pos })?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(UassetError::Truncated { offset: self.pos })?;
        self.pos = end;
        Ok(slice)
    }

    pub fn u8(&mut self) -> Result<u8, UassetError> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, UassetError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, UassetError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, UassetError> {
        Ok(self.u32()? as i32)
    }

    pub fn i64(&mut self) -> Result<i64, UassetError> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes(b.try_into().expect("took exactly 8 bytes")))
    }

    pub fn f32(&mut self) -> Result<f32, UassetError> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn f64(&mut self) -> Result<f64, UassetError> {
        let b = self.take(8)?;
        Ok(f64::from_le_bytes(b.try_into().expect("took exactly 8 bytes")))
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn skip(&mut self, n: usize) -> Result<(), UassetError> {
        self.take(n).map(|_| ())
    }

    /// Unreal `FString`: length-prefixed, NUL-terminated. A positive length is
    /// a byte count of ASCII/UTF-8; a negative length is a UTF-16 *character*
    /// count.
    pub fn fstring(&mut self) -> Result<String, UassetError> {
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

/// One entry of the import map — an object this package refers to but does not
/// contain.
#[derive(Debug, Clone)]
pub struct Import {
    pub class_package: String,
    pub class_name: String,
    pub outer_index: i32,
    pub object_name: String,
}

/// One entry of the export map — an object this package contains.
#[derive(Debug, Clone)]
pub struct Export {
    /// `FPackageIndex` of this object's class: negative selects an import,
    /// positive an export, zero is null. Resolve with [`Package::class_of`].
    pub class_index: i32,
    pub object_name: String,
    /// Length of this export's serialized data.
    pub serial_size: u64,
    /// Offset of that data within the *virtual* package — header and `.uexp`
    /// concatenated. Subtract [`Package::total_header_size`] for an offset
    /// into the `.uexp`; [`Package::uexp_offset`] does this.
    pub serial_offset: u64,
}

/// Size of one `FObjectExport` record as cooked by UE 5.1.
///
/// Validated against the real packages rather than assumed: the export map is
/// followed immediately by the depends map, and
/// `DependsOffset - ExportOffset` is exactly `96 * ExportCount`. A future
/// engine version that changes the record shape will fail
/// [`UassetError::ExportMapStride`] instead of silently mis-reading offsets.
const EXPORT_RECORD_SIZE: usize = 96;

/// Size of one `FObjectImport` record as cooked by UE 5.1 — the classic 28
/// bytes plus the UE5 `bImportOptional` flag.
const IMPORT_RECORD_SIZE: usize = 32;

/// `EPackageFlags::PKG_FilterEditorOnly`. When set (as it is for every cooked
/// package here) the summary omits its `LocalizationId` string.
const PKG_FILTER_EDITOR_ONLY: u32 = 0x8000_0000;

/// A cooked package's header: name table plus the import and export maps.
///
/// Note that Palworld's packages are cooked *unversioned* — every file-version
/// field in the summary reads 0 — so the layout below cannot be selected by
/// version gating the way a normal `.uasset` reader would. It is instead fixed
/// to the UE 5.1 shape the game ships and validated structurally.
#[derive(Debug, Clone)]
pub struct Package {
    pub summary: PackageSummary,
    pub total_header_size: i32,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
}

impl Package {
    /// Where an export's data starts within the `.uexp`.
    #[must_use]
    pub fn uexp_offset(&self, export: &Export) -> Option<usize> {
        export
            .serial_offset
            .checked_sub(self.total_header_size.max(0) as u64)
            .and_then(|o| usize::try_from(o).ok())
    }

    /// Resolve an export's class name through the import map.
    #[must_use]
    pub fn class_of(&self, export: &Export) -> Option<&str> {
        let index = export.class_index;
        if index >= 0 {
            return None;
        }
        let slot = usize::try_from(-index - 1).ok()?;
        self.imports.get(slot).map(|i| i.object_name.as_str())
    }

    /// Resolve an `FPackageIndex` to an import's object name, or `None` if it
    /// does not point at one.
    #[must_use]
    pub fn import_name(&self, package_index: i32) -> Option<&str> {
        if package_index >= 0 {
            return None;
        }
        let slot = usize::try_from(-package_index - 1).ok()?;
        self.imports.get(slot).map(|i| i.object_name.as_str())
    }

    /// The first export whose class is `class_name`.
    #[must_use]
    pub fn find_export(&self, class_name: &str) -> Option<&Export> {
        self.exports.iter().find(|e| self.class_of(e) == Some(class_name))
    }
}

/// Parse a cooked `.uasset` package header, including its import and export
/// maps.
pub fn parse_package(bytes: &[u8]) -> Result<Package, UassetError> {
    let (summary, mut r, total_header_size) = parse_summary_inner(bytes)?;

    // Soft object paths (UE5 `ADD_SOFTOBJECTPATH_LIST`), then the optional
    // localization id, then the gatherable text map — all skipped.
    r.skip(8)?;
    if summary.package_flags & PKG_FILTER_EDITOR_ONLY == 0 {
        r.fstring()?;
    }
    r.skip(8)?;

    let export_count = r.i32()?;
    let export_offset = r.i32()?;
    let import_count = r.i32()?;
    let import_offset = r.i32()?;
    let depends_offset = r.i32()?;

    let names = &summary.names;

    let mut imports = Vec::new();
    if import_count > 0 {
        let count = usize::try_from(import_count)
            .map_err(|_| UassetError::Truncated { offset: r.pos() })?;
        let mut ir = Reader::at(bytes, usize::try_from(import_offset).unwrap_or(0));
        imports.reserve(count);
        for _ in 0..count {
            let start = ir.pos();
            let class_package = read_fname(&mut ir, names)?;
            let class_name = read_fname(&mut ir, names)?;
            let outer_index = ir.i32()?;
            let object_name = read_fname(&mut ir, names)?;
            imports.push(Import { class_package, class_name, outer_index, object_name });
            // Skip `bImportOptional` and anything else in the record.
            ir.seek(start + IMPORT_RECORD_SIZE);
        }
    }

    let mut exports = Vec::new();
    if export_count > 0 {
        let count = usize::try_from(export_count)
            .map_err(|_| UassetError::Truncated { offset: r.pos() })?;
        // The depends map begins where the export map ends, which pins the
        // record size without trusting a hard-coded constant blindly.
        if depends_offset > export_offset {
            let span = (depends_offset - export_offset) as usize;
            if span != count * EXPORT_RECORD_SIZE {
                return Err(UassetError::ExportMapStride {
                    span,
                    count,
                    expected: EXPORT_RECORD_SIZE,
                });
            }
        }
        let base = usize::try_from(export_offset).unwrap_or(0);
        exports.reserve(count);
        for i in 0..count {
            let start = base + i * EXPORT_RECORD_SIZE;
            let mut er = Reader::at(bytes, start);
            let class_index = er.i32()?;
            er.skip(12)?; // SuperIndex, TemplateIndex, OuterIndex
            let object_name = read_fname(&mut er, names)?;
            er.skip(4)?; // ObjectFlags
            let serial_size = er.i64()?;
            let serial_offset = er.i64()?;
            exports.push(Export {
                class_index,
                object_name,
                serial_size: serial_size.max(0) as u64,
                serial_offset: serial_offset.max(0) as u64,
            });
        }
    }

    Ok(Package { summary, total_header_size, imports, exports })
}

/// An `FName` as stored in a package: an index into the name table plus an
/// occurrence number.
fn read_fname(r: &mut Reader, names: &[String]) -> Result<String, UassetError> {
    let index = r.i32()?;
    let _number = r.i32()?;
    usize::try_from(index)
        .ok()
        .and_then(|i| names.get(i))
        .cloned()
        .ok_or(UassetError::BadNameIndex { index, count: names.len() })
}

/// Parse a cooked `.uasset` package header and its name table.
pub fn parse_summary(bytes: &[u8]) -> Result<PackageSummary, UassetError> {
    parse_summary_inner(bytes).map(|(s, _, _)| s)
}

/// Shared prefix of [`parse_summary`] and [`parse_package`]: everything up to
/// and including `NameOffset`, leaving the cursor positioned for the rest.
fn parse_summary_inner(bytes: &[u8]) -> Result<(PackageSummary, Reader<'_>, i32), UassetError> {
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

    let total_header_size = r.i32()?;
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

    Ok((PackageSummary { package_flags, folder_name, names }, r, total_header_size))
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
