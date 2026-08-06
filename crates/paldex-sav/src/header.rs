use crate::SavError;

/// Byte length of the container header.
pub const HEADER_LEN: usize = 12;

/// Which compression scheme wrapped the `GVAS` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Oodle Kraken/Leviathan/Mermaid/Selkie, magic `PlM`. The only scheme
    /// observed on real saves — Palworld 0.6+ writes nothing else.
    Oodle,
    /// Single-pass zlib, magic `PlZ`, save type `0x31`.
    Zlib1,
    /// Double-pass zlib, magic `PlZ`, save type `0x32`. Retained for
    /// older/imported saves; no real `PlZ` file exists on this machine to
    /// verify against, only synthetic fixtures.
    Zlib2,
}

/// The 12-byte container header preceding every `.sav` payload.
#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub uncompressed_len: u32,
    pub compressed_len: u32,
    pub compression: Compression,
}

impl Header {
    /// Parse and validate the header from the start of a `.sav` file's bytes.
    ///
    /// Validates the declared uncompressed length against `max_uncompressed_len`
    /// and the declared compressed length against what's actually available in
    /// `raw`, so a corrupt or truncated file is rejected here rather than during
    /// decompression.
    pub fn parse(raw: &[u8], max_uncompressed_len: u32) -> Result<Self, SavError> {
        let Some(head) = raw.get(..HEADER_LEN) else {
            return Err(SavError::TooShort(raw.len()));
        };

        let uncompressed_len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let compressed_len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
        let magic = [head[8], head[9], head[10]];
        let save_type = head[11];

        if magic == *b"CNK" {
            return Err(SavError::UnsupportedXboxContainer);
        }

        let compression = match &magic {
            b"PlM" => Compression::Oodle,
            b"PlZ" => match save_type {
                0x31 => Compression::Zlib1,
                0x32 => Compression::Zlib2,
                other => return Err(SavError::UnknownSaveType(other)),
            },
            _ => return Err(SavError::UnknownMagic(magic)),
        };

        if uncompressed_len > max_uncompressed_len {
            return Err(SavError::OversizedPayload {
                declared: uncompressed_len,
                limit: max_uncompressed_len,
            });
        }

        let available = raw.len() - HEADER_LEN;
        if compressed_len as usize > available {
            return Err(SavError::LengthMismatch {
                declared: compressed_len,
                available,
            });
        }

        Ok(Header {
            uncompressed_len,
            compressed_len,
            compression,
        })
    }
}
