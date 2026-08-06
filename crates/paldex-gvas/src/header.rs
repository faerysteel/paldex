use serde::Serialize;

use crate::cursor::Cursor;
use crate::GvasError;

/// Guard against a corrupt file claiming an absurd custom-version count before
/// allocating a `Vec` for it. 85 is what real saves carry; this leaves generous
/// headroom for future engine versions without accepting garbage.
const MAX_CUSTOM_VERSIONS: u32 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub changelist: u32,
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CustomVersion {
    pub key: [u8; 16],
    pub version: i32,
}

/// The fixed-shape header every `GVAS` file starts with, verified byte-for-byte
/// against a real decompressed `LevelMeta.sav` on this machine: magic, a save-game
/// version, a UE4/UE5 package version pair, the engine version that wrote the file,
/// a custom-format version, a list of per-system custom versions, and finally the
/// save-game class name. What follows the header is the property list (see
/// [`crate::value`]).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Header {
    pub save_game_version: u32,
    pub package_file_version_ue4: u32,
    pub package_file_version_ue5: u32,
    pub engine_version: EngineVersion,
    pub custom_format_version: u32,
    pub custom_versions: Vec<CustomVersion>,
    pub save_game_class_name: String,
}

impl Header {
    pub fn parse(cursor: &mut Cursor) -> Result<Self, GvasError> {
        let magic = cursor.bytes(4)?;
        if magic != b"GVAS" {
            return Err(GvasError::NotGvas);
        }

        let save_game_version = cursor.u32()?;
        let package_file_version_ue4 = cursor.u32()?;
        let package_file_version_ue5 = cursor.u32()?;

        let major = cursor.u16()?;
        let minor = cursor.u16()?;
        let patch = cursor.u16()?;
        let changelist = cursor.u32()?;
        let branch = cursor.fstring()?;
        let engine_version = EngineVersion {
            major,
            minor,
            patch,
            changelist,
            branch,
        };

        let custom_format_version = cursor.u32()?;
        let count = cursor.u32()?;
        if count > MAX_CUSTOM_VERSIONS {
            return Err(GvasError::ImplausibleCount {
                declared: count,
                limit: MAX_CUSTOM_VERSIONS,
                what: "custom format versions",
            });
        }
        let mut custom_versions = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let key = cursor.guid()?;
            let version = cursor.i32()?;
            custom_versions.push(CustomVersion { key, version });
        }

        let save_game_class_name = cursor.fstring()?;

        Ok(Header {
            save_game_version,
            package_file_version_ue4,
            package_file_version_ue5,
            engine_version,
            custom_format_version,
            custom_versions,
            save_game_class_name,
        })
    }
}
