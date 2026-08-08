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

pub mod datatable;
mod extract;
mod reference;
pub mod text_table;
pub mod texture;
pub mod uasset;
pub mod unversioned;
pub mod usmap;
pub use extract::{ExtractError, ReferenceIndex, TEXT_LANGUAGES};

/// The property schema for Palworld's cooked packages, bundled because the
/// game ships none of its own.
///
/// Regenerate with `tools/usmap/regen-usmap.sh` after a game update; see that
/// directory's README for why the stock mappings dumper needs patching for
/// this build.
pub const BUNDLED_MAPPINGS: &[u8] = include_bytes!("../data/Mappings.usmap");
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

/// Decode a texture from the pak into a PNG.
///
/// `base` is a pak entry path without its extension, as returned by
/// [`ReferenceData::icon_path`]. Mip 0 usually lives in a sibling `.ubulk`,
/// which is read when present.
///
/// # Errors
///
/// Fails if the entries can't be read, the texture can't be located, or its
/// pixel format isn't one of the supported block-compressed formats.
pub fn load_icon_png(pak: &mut Pak, base: &str) -> Result<Vec<u8>, IconError> {
    let uexp = pak.read(&format!("{base}.uexp"))?;
    // Absent `.ubulk` is normal: small textures inline every mip.
    let ubulk = pak.read(&format!("{base}.ubulk")).ok();

    let info = texture::parse(&uexp)?;
    let data = texture::mip0_bytes(&info, &uexp, ubulk.as_deref())?;
    let rgba = texture::decode_rgba(&info, data)?;
    Ok(texture::encode_png(info.width, info.height, &rgba)?)
}

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error(transparent)]
    Pak(#[from] PakError),
    #[error(transparent)]
    Texture(#[from] texture::TextureError),
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
