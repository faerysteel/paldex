//! Reads the game's own installed `.pak` file for reference data extraction —
//! species names, elements, work suitabilities, passive-skill definitions,
//! and icon artwork — at build/extraction time, from the user's own install.
//!
//! Verified against the real `Pal-Windows.pak` on this machine: version 11,
//! magic `0x5A6F12E1`, `bEncryptedIndex = 0` (no AES key needed), mount point
//! `../../../`, 185,003 entries. Uses [`repak_oodle`] (a vendored, patched
//! copy of `trumank/repak` — see that crate's `NOTICE.md`) for the pak
//! container format, and `oozextract` (via `repak_oodle`) for Oodle
//! decompression, the same decoder already verified in `paldex-sav`.

use std::fs::File;
use std::io::{BufReader, Seek};
use std::path::Path;

mod reference;
pub use reference::{PassiveSkill, PassthroughReferenceData, ReferenceData, Species};
pub use repak_oodle::{PakBuilder, PakReader};

#[derive(Debug, thiserror::Error)]
pub enum PakError {
    #[error("opening pak file {path}: {source}")]
    Open {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("reading pak index: {0}")]
    Index(#[source] repak_oodle::Error),
    #[error("reading entry {path:?}: {source}")]
    Entry {
        path: String,
        #[source]
        source: repak_oodle::Error,
    },
}

/// An opened pak archive, ready for path-based entry lookups.
pub struct Pak {
    reader: PakReader,
    file: BufReader<File>,
}

impl Pak {
    /// Open and index a `.pak` file.
    pub fn open(path: &Path) -> Result<Self, PakError> {
        let file = File::open(path).map_err(|source| PakError::Open {
            path: path.display().to_string(),
            source,
        })?;
        let mut file = BufReader::new(file);
        let reader = PakBuilder::new().reader(&mut file).map_err(PakError::Index)?;
        Ok(Self { reader, file })
    }

    #[must_use]
    pub fn version(&self) -> repak_oodle::Version {
        self.reader.version()
    }

    #[must_use]
    pub fn mount_point(&self) -> &str {
        self.reader.mount_point()
    }

    #[must_use]
    pub fn encrypted_index(&self) -> bool {
        self.reader.encrypted_index()
    }

    /// Every entry path in the archive.
    #[must_use]
    pub fn files(&self) -> Vec<String> {
        self.reader.files()
    }

    /// Read and decompress a single entry by its path within the pak.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, PakError> {
        self.file.rewind().map_err(|source| PakError::Open {
            path: path.to_owned(),
            source,
        })?;
        self.reader
            .get(path, &mut self.file)
            .map_err(|source| PakError::Entry {
                path: path.to_owned(),
                source,
            })
    }
}
