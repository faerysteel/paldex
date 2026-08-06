//! Palworld `.sav` container layer: header parsing and payload decompression.
//!
//! Palworld saves are a small custom container wrapping a `GVAS`-format
//! Unreal Engine SaveGame blob. The 12-byte header (all little-endian):
//!
//! | offset | size | field                     |
//! |--------|------|---------------------------|
//! | 0      | 4    | uncompressed length       |
//! | 4      | 4    | compressed length         |
//! | 8      | 3    | magic (`PlM`/`PlZ`/`CNK`) |
//! | 11     | 1    | save type                 |
//!
//! Verified against every real save file on this machine — `Level.sav`,
//! `LevelMeta.sav`, `LocalData.sav`, `WorldOption.sav`, and both
//! `Players/<uid>.sav` and `Players/<uid>_dps.sav` — all of which use magic
//! `PlM` (Oodle) with save type `0x31`. See [`Compression`] for the caveat
//! on zlib support.

use std::io::Read;

use flate2::read::ZlibDecoder;

mod header;

pub use header::{Compression, Header, HEADER_LEN};

/// Reject any declared uncompressed length above this many bytes, by default.
///
/// Guards against `_dps.sav`-style decompression bombs (42 KB compresses to
/// 73 MB — a ~1,740x ratio) driving an unbounded allocation.
pub const DEFAULT_MAX_UNCOMPRESSED_LEN: u32 = 256 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SavError {
    #[error("input is {0} bytes, shorter than the 12-byte container header")]
    TooShort(usize),

    #[error("declared uncompressed length {declared} exceeds the {limit}-byte ceiling")]
    OversizedPayload { declared: u32, limit: u32 },

    #[error("unrecognized container magic {0:?}")]
    UnknownMagic([u8; 3]),

    #[error("Xbox/Game Pass CNK save container is not supported")]
    UnsupportedXboxContainer,

    #[error("unrecognized zlib save type byte 0x{0:02X}")]
    UnknownSaveType(u8),

    #[error(
        "declared compressed length {declared} exceeds the {available} bytes available after the header"
    )]
    LengthMismatch { declared: u32, available: usize },

    #[error("zlib decompression failed: {0}")]
    Zlib(#[from] std::io::Error),

    #[error("decompressed length {actual} does not match the declared length {declared}")]
    LengthDeclaredMismatch { declared: u32, actual: usize },

    #[error("Oodle decompression failed")]
    Oodle,

    #[error("decompressed payload does not start with the GVAS magic")]
    NotGvas,
}

/// Decompress a `.sav` container's bytes into its `GVAS` payload.
///
/// Uses [`DEFAULT_MAX_UNCOMPRESSED_LEN`] as the oversized-payload ceiling.
pub fn decompress(raw: &[u8]) -> Result<(Vec<u8>, Compression), SavError> {
    decompress_with_limit(raw, DEFAULT_MAX_UNCOMPRESSED_LEN)
}

/// Decompress a `.sav` container's bytes, rejecting a declared uncompressed
/// length above `max_uncompressed_len` before any allocation is made.
pub fn decompress_with_limit(
    raw: &[u8],
    max_uncompressed_len: u32,
) -> Result<(Vec<u8>, Compression), SavError> {
    let header = Header::parse(raw, max_uncompressed_len)?;
    let payload = &raw[HEADER_LEN..HEADER_LEN + header.compressed_len as usize];

    let out = match header.compression {
        Compression::Oodle => decompress_oodle(payload, header.uncompressed_len)?,
        Compression::Zlib1 => zlib_decompress(payload)?,
        Compression::Zlib2 => zlib_decompress(&zlib_decompress(payload)?)?,
    };

    if out.len() != header.uncompressed_len as usize {
        return Err(SavError::LengthDeclaredMismatch {
            declared: header.uncompressed_len,
            actual: out.len(),
        });
    }
    if !out.starts_with(b"GVAS") {
        return Err(SavError::NotGvas);
    }

    Ok((out, header.compression))
}

fn decompress_oodle(payload: &[u8], uncompressed_len: u32) -> Result<Vec<u8>, SavError> {
    let mut out = vec![0u8; uncompressed_len as usize];
    oozextract::Extractor::new()
        .read_from_slice(payload, &mut out)
        .map_err(|_| SavError::Oodle)?;
    Ok(out)
}

fn zlib_decompress(payload: &[u8]) -> Result<Vec<u8>, SavError> {
    let mut out = Vec::new();
    ZlibDecoder::new(payload).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression as FlateCompression;
    use std::io::Write;

    fn header(uncompressed_len: u32, compressed_len: u32, magic: &[u8; 3], save_type: u8) -> Vec<u8> {
        let mut h = Vec::with_capacity(HEADER_LEN);
        h.extend_from_slice(&uncompressed_len.to_le_bytes());
        h.extend_from_slice(&compressed_len.to_le_bytes());
        h.extend_from_slice(magic);
        h.push(save_type);
        h
    }

    fn zlib_compress(data: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), FlateCompression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn too_short_input_is_rejected() {
        let err = decompress(&[0u8; 4]).unwrap_err();
        assert!(matches!(err, SavError::TooShort(4)));
    }

    #[test]
    fn unknown_magic_is_rejected() {
        let mut raw = header(0, 0, b"XXX", 0);
        raw.extend_from_slice(b"");
        let err = decompress(&raw).unwrap_err();
        assert!(matches!(err, SavError::UnknownMagic(m) if &m == b"XXX"));
    }

    #[test]
    fn cnk_container_is_explicitly_unsupported() {
        let raw = header(0, 0, b"CNK", 0);
        let err = decompress(&raw).unwrap_err();
        assert!(matches!(err, SavError::UnsupportedXboxContainer));
    }

    #[test]
    fn unknown_zlib_save_type_is_rejected() {
        let raw = header(0, 0, b"PlZ", 0x99);
        let err = decompress(&raw).unwrap_err();
        assert!(matches!(err, SavError::UnknownSaveType(0x99)));
    }

    #[test]
    fn oversized_declared_length_is_rejected_before_allocating() {
        let raw = header(u32::MAX, 0, b"PlM", 0x31);
        let err = decompress_with_limit(&raw, 1024).unwrap_err();
        assert!(matches!(
            err,
            SavError::OversizedPayload {
                declared: u32::MAX,
                limit: 1024
            }
        ));
    }

    #[test]
    fn truncated_payload_is_rejected_by_length_check() {
        let mut raw = header(100, 100, b"PlM", 0x31);
        raw.extend_from_slice(&[0u8; 10]); // far short of the declared 100
        let err = decompress(&raw).unwrap_err();
        assert!(matches!(
            err,
            SavError::LengthMismatch {
                declared: 100,
                available: 10
            }
        ));
    }

    #[test]
    fn single_pass_zlib_round_trips() {
        let gvas = b"GVAS-single-pass-payload";
        let compressed = zlib_compress(gvas);
        let mut raw = header(gvas.len() as u32, compressed.len() as u32, b"PlZ", 0x31);
        raw.extend_from_slice(&compressed);

        let (out, compression) = decompress(&raw).unwrap();
        assert_eq!(out, gvas);
        assert_eq!(compression, Compression::Zlib1);
    }

    #[test]
    fn double_pass_zlib_round_trips() {
        let gvas = b"GVAS-double-pass-payload";
        let once = zlib_compress(gvas);
        let twice = zlib_compress(&once);
        let mut raw = header(gvas.len() as u32, twice.len() as u32, b"PlZ", 0x32);
        raw.extend_from_slice(&twice);

        let (out, compression) = decompress(&raw).unwrap();
        assert_eq!(out, gvas);
        assert_eq!(compression, Compression::Zlib2);
    }

    #[test]
    fn zlib_payload_not_starting_with_gvas_is_rejected() {
        let payload = b"not-a-gvas-file-at-all";
        let compressed = zlib_compress(payload);
        let mut raw = header(payload.len() as u32, compressed.len() as u32, b"PlZ", 0x31);
        raw.extend_from_slice(&compressed);

        let err = decompress(&raw).unwrap_err();
        assert!(matches!(err, SavError::NotGvas));
    }

    #[test]
    fn declared_length_mismatch_after_zlib_decompress_is_caught() {
        let gvas = b"GVASxxxxxxxxxxxxxxxxxxxx";
        let compressed = zlib_compress(gvas);
        // Lie about the uncompressed length.
        let mut raw = header((gvas.len() + 5) as u32, compressed.len() as u32, b"PlZ", 0x31);
        raw.extend_from_slice(&compressed);

        let err = decompress(&raw).unwrap_err();
        assert!(matches!(err, SavError::LengthDeclaredMismatch { .. }));
    }
}
