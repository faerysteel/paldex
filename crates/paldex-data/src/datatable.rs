//! Reads cooked `UDataTable` rows using a `.usmap` schema.
//!
//! This is what finally makes the numeric game data reachable: base stats,
//! elements, work suitabilities, rarity and Paldeck numbers all live in
//! `DataTable` rows whose values are serialized without names or types.
//!
//! ## Export layout
//!
//! An export's bytes, all established by reading the real packages rather than
//! inferred from engine version gates (which read 0 in these cooked files):
//!
//! ```text
//! <unversioned properties of the export's own class>   e.g. RowStruct, ParentTables
//! i32  bHasObjectGuid        UE serializes archive bools as 4 bytes
//! FGuid ObjectGuid           only when the flag above is non-zero
//! i32  NumRows
//! NumRows * { FName RowName ; <unversioned properties of the row struct> }
//! ```
//!
//! ## Why this is trustworthy
//!
//! Unversioned values are untagged, so a single wrong field size desynchronizes
//! everything after it. That makes the format self-checking: [`read`] requires
//! the walk to finish on the **exact** byte the export map declares, and
//! errors otherwise. Both real tables satisfy this — `DT_PalMonsterParameter`
//! consumes exactly 186,033 bytes and `DT_PalMonsterParameter_Common` exactly
//! 186,023, 753 rows each.

use crate::uasset::{self, Reader};
use crate::unversioned::{Properties, UnversionedError, UnversionedReader};
use crate::usmap::Usmap;

#[derive(Debug, thiserror::Error)]
pub enum DataTableError {
    #[error("parsing the package header: {0}")]
    Header(#[from] uasset::UassetError),
    #[error("decoding row properties: {0}")]
    Row(#[from] UnversionedError),
    #[error("the package contains no DataTable export")]
    NoDataTableExport,
    #[error("the DataTable export names no row struct")]
    NoRowStruct,
    #[error(
        "row walk ended at byte {actual} but the export declares {expected} — \
         the decoded layout does not match the data"
    )]
    LengthMismatch { actual: usize, expected: usize },
    #[error("export data at {offset} is outside the {len}-byte .uexp")]
    ExportOutOfRange { offset: usize, len: usize },
    #[error("implausible row count {0}")]
    BadRowCount(i32),
}

/// Classes whose exports are data tables. Palworld's parameter tables are
/// `CompositeDataTable`s that compose a plain `DataTable` sibling.
const DATA_TABLE_CLASSES: &[&str] = &["DataTable", "CompositeDataTable"];

/// Refuse a row count beyond this. Real tables are in the hundreds; a wild
/// value means we have desynced and should error rather than allocate.
const MAX_ROWS: i32 = 1_000_000;

/// One decoded table row.
#[derive(Debug, Clone)]
pub struct Row {
    pub name: String,
    pub properties: Properties,
}

/// A decoded `UDataTable`.
#[derive(Debug, Clone)]
pub struct DataTable {
    /// The struct every row conforms to, e.g.
    /// `PalCharacterParameterDatabaseRow`.
    pub row_struct: String,
    /// Rows in file order. A table may repeat a row name — the real
    /// `DT_PalMonsterParameter` has 753 rows under 735 distinct names — so
    /// this is deliberately a list, not a map; later rows win, matching the
    /// engine's own `TMap` insert.
    pub rows: Vec<Row>,
}

impl DataTable {
    /// The last row with this name, which is the one the engine would keep.
    #[must_use]
    pub fn row(&self, name: &str) -> Option<&Row> {
        self.rows.iter().rev().find(|r| r.name == name)
    }
}

/// Decode the `DataTable` export of a cooked package.
///
/// `uasset` is the package header and `uexp` its export data — the two halves
/// Unreal cooks a package into.
pub fn read(uasset: &[u8], uexp: &[u8], usmap: &Usmap) -> Result<DataTable, DataTableError> {
    let package = uasset::parse_package(uasset)?;

    let export = DATA_TABLE_CLASSES
        .iter()
        .find_map(|class| package.find_export(class))
        .ok_or(DataTableError::NoDataTableExport)?;
    let class = package.class_of(export).unwrap_or("DataTable");

    let offset = package
        .uexp_offset(export)
        .filter(|o| *o <= uexp.len())
        .ok_or(DataTableError::ExportOutOfRange {
            offset: export.serial_offset as usize,
            len: uexp.len(),
        })?;
    let end = offset
        .checked_add(export.serial_size as usize)
        .filter(|e| *e <= uexp.len())
        .ok_or(DataTableError::ExportOutOfRange { offset, len: uexp.len() })?;

    let reader = UnversionedReader::new(usmap, &package.summary.names);
    let mut r = Reader::at(uexp, offset);

    // The table object's own properties — `RowStruct`, and for a composite
    // table the `ParentTables` it merges.
    let object = reader.read_struct(&mut r, class)?;

    // `UObject::Serialize` writes a bool for "do I carry a GUID" after the
    // script properties; UE serializes archive bools as int32.
    if r.i32()? != 0 {
        r.skip(16)?;
    }

    let row_struct = object
        .get("RowStruct")
        .and_then(|v| match v {
            crate::unversioned::Value::Object(index) => package.import_name(*index),
            _ => None,
        })
        .ok_or(DataTableError::NoRowStruct)?
        .to_owned();

    let row_count = r.i32()?;
    if !(0..=MAX_ROWS).contains(&row_count) {
        return Err(DataTableError::BadRowCount(row_count));
    }

    let mut rows = Vec::with_capacity(row_count.min(4096) as usize);
    for _ in 0..row_count {
        let name_index = r.i32()?;
        let _number = r.i32()?;
        let name = usize::try_from(name_index)
            .ok()
            .and_then(|i| package.summary.names.get(i))
            .cloned()
            .unwrap_or_default();
        let properties = reader.read_struct(&mut r, &row_struct)?;
        rows.push(Row { name, properties });
    }

    // The whole point of the exactness check: untagged values give no other
    // signal that a field size was wrong.
    if r.pos() != end {
        return Err(DataTableError::LengthMismatch { actual: r.pos(), expected: end });
    }

    Ok(DataTable { row_struct, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_row_wins_for_a_duplicated_name() {
        let table = DataTable {
            row_struct: "R".into(),
            rows: vec![
                Row { name: "dup".into(), properties: Properties::new() },
                Row {
                    name: "dup".into(),
                    properties: Properties::from([(
                        "Hp".to_owned(),
                        crate::unversioned::Value::Int(9),
                    )]),
                },
            ],
        };
        assert_eq!(
            table.row("dup").and_then(|r| r.properties.get("Hp")),
            Some(&crate::unversioned::Value::Int(9))
        );
        assert!(table.row("missing").is_none());
    }

    #[test]
    fn rejects_a_package_that_is_not_a_datatable() {
        let usmap = Usmap::default();
        let err = read(&[0u8; 4], &[], &usmap).unwrap_err();
        assert!(matches!(err, DataTableError::Header(_)));
    }
}
