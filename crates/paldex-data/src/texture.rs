//! Locates the top mip of a cooked `UTexture2D` without a `.usmap`.
//!
//! Like every other cooked package, a texture's *properties* are unversioned
//! and unreadable without a schema. But `FTexturePlatformData` — which is what
//! actually holds the pixel data — is serialized after them and writes its
//! pixel format as a plain `FString` (`PF_DXT5`, `PF_DXT1`, `PF_BC7`, …). That
//! string is findable by byte search, and `SizeX`/`SizeY`/`PackedData` sit in
//! the twelve bytes immediately before it.
//!
//! Layout after the format name, verified byte-for-byte against the real
//! `T_Anubis_icon_normal` (128×128 `PF_DXT5`):
//!
//! ```text
//! i32  FirstMipToSerialize
//! i32  NumMips
//! per mip (FTexture2DMipMap):
//!   u32  BulkDataFlags
//!   i32  ElementCount          // == byte length for block-compressed data
//!   i32  BulkDataSizeOnDisk
//!   i64  BulkDataOffsetInFile
//!   [inline payload]           // present iff BULKDATA_ForceInlinePayload
//!   i32  SizeX, SizeY, SizeZ
//! ```
//!
//! Mip 0 normally carries `BULKDATA_PayloadInSeperateFile`, meaning its bytes
//! live in the sibling `.ubulk` at `BulkDataOffsetInFile`; smaller mips are
//! inlined in the `.uexp`.

use crate::uasset::{Reader, UassetError};

/// `BULKDATA_ForceInlinePayload` — payload follows the header in the `.uexp`.
const BULKDATA_FORCE_INLINE_PAYLOAD: u32 = 0x0040;
/// `BULKDATA_PayloadInSeperateFile` — payload lives in the sibling `.ubulk`.
const BULKDATA_PAYLOAD_IN_SEPARATE_FILE: u32 = 0x0100;

/// Block-compressed pixel formats. Only the ones Palworld's icons actually
/// use are represented; anything else is reported as [`TextureError::
/// UnsupportedFormat`] rather than silently mis-decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// BC1 — 8 bytes per 4×4 block.
    Dxt1,
    /// BC2 — 16 bytes per 4×4 block, sharp alpha.
    Dxt3,
    /// BC3 — 16 bytes per 4×4 block, interpolated alpha.
    Dxt5,
    /// Uncompressed 8-bit BGRA.
    B8G8R8A8,
}

impl PixelFormat {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "PF_DXT1" => Some(Self::Dxt1),
            "PF_DXT3" => Some(Self::Dxt3),
            "PF_DXT5" => Some(Self::Dxt5),
            "PF_B8G8R8A8" => Some(Self::B8G8R8A8),
            _ => None,
        }
    }

    /// Bytes one 4×4 block occupies, for block-compressed formats.
    fn block_bytes(self) -> Option<usize> {
        match self {
            Self::Dxt1 => Some(8),
            Self::Dxt3 | Self::Dxt5 => Some(16),
            Self::B8G8R8A8 => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TextureError {
    #[error("no FTexturePlatformData pixel format found — not a cooked texture?")]
    NoPlatformData,
    #[error("unsupported pixel format {0}")]
    UnsupportedFormat(String),
    #[error("implausible texture dimensions {width}x{height}")]
    BadDimensions { width: i32, height: i32 },
    #[error("texture has no mips")]
    NoMips,
    #[error(transparent)]
    Read(#[from] UassetError),
    #[error("mip data runs past the end of {source_name} (need {needed} bytes at {offset})")]
    MipOutOfRange { source_name: &'static str, offset: usize, needed: usize },
    #[error("encoding PNG: {0}")]
    Encode(String),
}

/// Where a mip's bytes live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipLocation {
    pub offset: usize,
    pub len: usize,
    /// `true` when the bytes are in the sibling `.ubulk` rather than the
    /// `.uexp`.
    pub in_bulk: bool,
}

/// The top mip of a cooked texture, located but not yet decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureInfo {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub mip0: MipLocation,
}

impl TextureInfo {
    /// Whether mip 0 lives in the sibling `.ubulk`.
    #[must_use]
    pub fn mip0_in_bulk(&self) -> bool {
        self.mip0.in_bulk
    }
}

// Kept as an inherent field-style accessor so example code reads naturally.
impl std::ops::Deref for TextureInfo {
    type Target = MipLocation;
    fn deref(&self) -> &Self::Target {
        &self.mip0
    }
}

/// Parse a cooked texture's `.uexp` and locate its largest mip.
pub fn parse(uexp: &[u8]) -> Result<TextureInfo, TextureError> {
    let (format_at, format_name) = find_pixel_format(uexp).ok_or(TextureError::NoPlatformData)?;
    let format = PixelFormat::parse(&format_name)
        .ok_or_else(|| TextureError::UnsupportedFormat(format_name.clone()))?;

    let width = i32::from_le_bytes(uexp[format_at - 12..format_at - 8].try_into().unwrap());
    let height = i32::from_le_bytes(uexp[format_at - 8..format_at - 4].try_into().unwrap());
    if width <= 0 || height <= 0 || width > 16384 || height > 16384 {
        return Err(TextureError::BadDimensions { width, height });
    }

    // Position just past the pixel-format FString.
    let mut r = Reader::at(uexp, format_at);
    r.fstring()?;
    r.skip(4)?; // FirstMipToSerialize
    let num_mips = r.i32()?;
    if num_mips <= 0 {
        return Err(TextureError::NoMips);
    }

    // Mip 0 is the largest and the only one needed.
    let flags = r.u32()?;
    let element_count = r.i32()?;
    let size_on_disk = r.i32()?;
    let offset_in_file = r.i64()?;
    let len = usize::try_from(size_on_disk.max(element_count)).unwrap_or(0);

    let mip0 = if flags & BULKDATA_PAYLOAD_IN_SEPARATE_FILE != 0 {
        MipLocation { offset: usize::try_from(offset_in_file).unwrap_or(0), len, in_bulk: true }
    } else if flags & BULKDATA_FORCE_INLINE_PAYLOAD != 0 {
        // Inline payloads follow the header immediately.
        MipLocation { offset: r.pos(), len, in_bulk: false }
    } else {
        MipLocation { offset: usize::try_from(offset_in_file).unwrap_or(0), len, in_bulk: false }
    };

    Ok(TextureInfo {
        width: width.unsigned_abs(),
        height: height.unsigned_abs(),
        format,
        mip0,
    })
}

/// Read mip 0's bytes out of whichever container holds them.
pub fn mip0_bytes<'a>(
    info: &TextureInfo,
    uexp: &'a [u8],
    ubulk: Option<&'a [u8]>,
) -> Result<&'a [u8], TextureError> {
    let (source, source_name) = if info.mip0.in_bulk {
        (ubulk.unwrap_or(&[]), "ubulk")
    } else {
        (uexp, "uexp")
    };
    let end = info.mip0.offset.saturating_add(info.mip0.len);
    source.get(info.mip0.offset..end).ok_or(TextureError::MipOutOfRange {
        source_name,
        offset: info.mip0.offset,
        needed: info.mip0.len,
    })
}

/// Find the `FString` holding the pixel format name, returning its offset and
/// value. Anchored on the `PF_` prefix, which no other string in these
/// packages uses at a plausible `FString` length.
fn find_pixel_format(bytes: &[u8]) -> Option<(usize, String)> {
    let mut i = 12usize; // SizeX/SizeY/PackedData must precede it
    while i + 8 <= bytes.len() {
        let len = i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        if (4..=64).contains(&len) {
            let end = i + 4 + len as usize;
            if end <= bytes.len() {
                let raw = &bytes[i + 4..end];
                if raw.starts_with(b"PF_") && raw.last() == Some(&0) {
                    let name = String::from_utf8_lossy(&raw[..raw.len() - 1]).into_owned();
                    return Some((i, name));
                }
            }
        }
        i += 1;
    }
    None
}

/// Decode a block-compressed mip to straight RGBA8.
pub fn decode_rgba(info: &TextureInfo, data: &[u8]) -> Result<Vec<u8>, TextureError> {
    let (w, h) = (info.width as usize, info.height as usize);
    let mut out = vec![0u8; w * h * 4];

    let Some(block_bytes) = info.format.block_bytes() else {
        // B8G8R8A8: swizzle to RGBA.
        for (px, chunk) in data.chunks_exact(4).take(w * h).enumerate() {
            out[px * 4] = chunk[2];
            out[px * 4 + 1] = chunk[1];
            out[px * 4 + 2] = chunk[0];
            out[px * 4 + 3] = chunk[3];
        }
        return Ok(out);
    };

    let blocks_x = w.div_ceil(4);
    let blocks_y = h.div_ceil(4);
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let idx = (by * blocks_x + bx) * block_bytes;
            let Some(block) = data.get(idx..idx + block_bytes) else {
                continue;
            };
            let (colour, alpha): (&[u8], Option<&[u8]>) = match info.format {
                PixelFormat::Dxt1 => (block, None),
                PixelFormat::Dxt3 | PixelFormat::Dxt5 => (&block[8..16], Some(&block[0..8])),
                PixelFormat::B8G8R8A8 => unreachable!("handled above"),
            };
            let mut texels = [[0u8; 4]; 16];
            decode_colour_block(colour, info.format == PixelFormat::Dxt1, &mut texels);
            match (info.format, alpha) {
                (PixelFormat::Dxt5, Some(a)) => decode_bc3_alpha(a, &mut texels),
                (PixelFormat::Dxt3, Some(a)) => decode_bc2_alpha(a, &mut texels),
                _ => {}
            }
            for (t, texel) in texels.iter().enumerate() {
                let (x, y) = (bx * 4 + t % 4, by * 4 + t / 4);
                if x < w && y < h {
                    let o = (y * w + x) * 4;
                    out[o..o + 4].copy_from_slice(texel);
                }
            }
        }
    }
    Ok(out)
}

/// BC1 colour block: two RGB565 endpoints plus 2-bit per-texel indices.
fn decode_colour_block(block: &[u8], punchthrough_alpha: bool, out: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let bits = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);

    let e0 = rgb565(c0);
    let e1 = rgb565(c1);
    // In BC1, c0 <= c1 selects the 3-colour mode where index 3 is transparent.
    let three_colour = punchthrough_alpha && c0 <= c1;
    let palette = [
        e0,
        e1,
        if three_colour { midpoint(e0, e1) } else { lerp_third(e0, e1) },
        if three_colour { [0, 0, 0, 0] } else { lerp_third(e1, e0) },
    ];

    for (i, texel) in out.iter_mut().enumerate() {
        *texel = palette[((bits >> (i * 2)) & 0b11) as usize];
    }
}

/// BC3 alpha block: two endpoints plus 3-bit per-texel indices.
fn decode_bc3_alpha(block: &[u8], out: &mut [[u8; 4]; 16]) {
    let a0 = block[0];
    let a1 = block[1];
    let mut palette = [0u8; 8];
    palette[0] = a0;
    palette[1] = a1;
    if a0 > a1 {
        for i in 0..6 {
            palette[i + 2] = (((6 - i) as u16 * a0 as u16 + (i + 1) as u16 * a1 as u16) / 7) as u8;
        }
    } else {
        for i in 0..4 {
            palette[i + 2] = (((4 - i) as u16 * a0 as u16 + (i + 1) as u16 * a1 as u16) / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }

    let bits = u64::from(block[2])
        | u64::from(block[3]) << 8
        | u64::from(block[4]) << 16
        | u64::from(block[5]) << 24
        | u64::from(block[6]) << 32
        | u64::from(block[7]) << 40;
    for (i, texel) in out.iter_mut().enumerate() {
        texel[3] = palette[((bits >> (i * 3)) & 0b111) as usize];
    }
}

/// BC2 alpha block: 4 bits per texel, no interpolation.
fn decode_bc2_alpha(block: &[u8], out: &mut [[u8; 4]; 16]) {
    for (i, texel) in out.iter_mut().enumerate() {
        let nibble = (block[i / 2] >> ((i % 2) * 4)) & 0x0F;
        texel[3] = nibble * 17; // 0..15 -> 0..255
    }
}

fn rgb565(c: u16) -> [u8; 4] {
    let r = ((c >> 11) & 0x1F) as u8;
    let g = ((c >> 5) & 0x3F) as u8;
    let b = (c & 0x1F) as u8;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2), 255]
}

fn midpoint(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    [
        ((a[0] as u16 + b[0] as u16) / 2) as u8,
        ((a[1] as u16 + b[1] as u16) / 2) as u8,
        ((a[2] as u16 + b[2] as u16) / 2) as u8,
        255,
    ]
}

/// `(2a + b) / 3`, the first interpolated endpoint in 4-colour mode.
fn lerp_third(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    [
        ((2 * a[0] as u16 + b[0] as u16) / 3) as u8,
        ((2 * a[1] as u16 + b[1] as u16) / 3) as u8,
        ((2 * a[2] as u16 + b[2] as u16) / 3) as u8,
        255,
    ]
}

/// Encode decoded RGBA8 pixels as a PNG.
///
/// PNG rather than raw pixels because the icons are ultimately rendered by a
/// webview `<img>`, and it keeps alpha intact — these are cut-out icons, so a
/// format without alpha would show black boxes.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, TextureError> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| TextureError::Encode(e.to_string()))?;
        writer.write_image_data(rgba).map_err(|e| TextureError::Encode(e.to_string()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the tail of a cooked texture: the 12 header bytes, the format
    /// name, and one inline mip.
    fn synthetic_texture(format: &str, w: i32, h: i32, payload: &[u8]) -> Vec<u8> {
        let mut b = vec![0xAAu8; 32]; // opaque unversioned properties
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&h.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes()); // PackedData
        b.extend_from_slice(&(format.len() as i32 + 1).to_le_bytes());
        b.extend_from_slice(format.as_bytes());
        b.push(0);
        b.extend_from_slice(&0i32.to_le_bytes()); // FirstMipToSerialize
        b.extend_from_slice(&1i32.to_le_bytes()); // NumMips
        b.extend_from_slice(&BULKDATA_FORCE_INLINE_PAYLOAD.to_le_bytes());
        b.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        b.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        b.extend_from_slice(&0i64.to_le_bytes());
        b.extend_from_slice(payload);
        b
    }

    #[test]
    fn locates_an_inline_mip() {
        let payload = vec![0u8; 16];
        let bytes = synthetic_texture("PF_DXT5", 4, 4, &payload);
        let info = parse(&bytes).expect("parse");
        assert_eq!(info.format, PixelFormat::Dxt5);
        assert_eq!((info.width, info.height), (4, 4));
        assert!(!info.mip0.in_bulk);
        assert_eq!(mip0_bytes(&info, &bytes, None).expect("mip"), &payload[..]);
    }

    #[test]
    fn rejects_unsupported_formats() {
        let bytes = synthetic_texture("PF_ASTC_4x4", 4, 4, &[0; 16]);
        assert!(matches!(parse(&bytes), Err(TextureError::UnsupportedFormat(_))));
    }

    #[test]
    fn rejects_input_without_platform_data() {
        assert!(matches!(parse(&[0u8; 64]), Err(TextureError::NoPlatformData)));
    }

    /// A DXT1 block with both endpoints white must decode to solid white.
    #[test]
    fn decodes_a_flat_dxt1_block() {
        let mut block = Vec::new();
        block.extend_from_slice(&0xFFFFu16.to_le_bytes()); // c0 = white
        block.extend_from_slice(&0xFFFFu16.to_le_bytes()); // c1 = white
        block.extend_from_slice(&0u32.to_le_bytes()); // all index 0
        let bytes = synthetic_texture("PF_DXT1", 4, 4, &block);
        let info = parse(&bytes).expect("parse");
        let rgba = decode_rgba(&info, mip0_bytes(&info, &bytes, None).expect("mip")).expect("decode");
        assert_eq!(rgba.len(), 4 * 4 * 4);
        assert!(rgba.chunks_exact(4).all(|p| p == [255, 255, 255, 255]));
    }

    /// BC3's alpha endpoints must reach the texels, not just the colour block.
    #[test]
    fn decodes_dxt5_alpha() {
        let mut block = vec![0u8; 16];
        block[0] = 255; // a0
        block[1] = 0; // a1
        // indices all 0 -> every texel takes a0
        block[8..10].copy_from_slice(&0xFFFFu16.to_le_bytes());
        block[10..12].copy_from_slice(&0xFFFFu16.to_le_bytes());
        let bytes = synthetic_texture("PF_DXT5", 4, 4, &block);
        let info = parse(&bytes).expect("parse");
        let rgba = decode_rgba(&info, mip0_bytes(&info, &bytes, None).expect("mip")).expect("decode");
        assert!(rgba.chunks_exact(4).all(|p| p[3] == 255), "alpha should be opaque");

        block[0] = 0;
        block[1] = 0;
        let bytes = synthetic_texture("PF_DXT5", 4, 4, &block);
        let info = parse(&bytes).expect("parse");
        let rgba = decode_rgba(&info, mip0_bytes(&info, &bytes, None).expect("mip")).expect("decode");
        assert!(rgba.chunks_exact(4).all(|p| p[3] == 0), "alpha should be transparent");
    }

    #[test]
    fn truncation_never_panics() {
        let full = synthetic_texture("PF_DXT5", 8, 8, &[0x5A; 64]);
        for n in 0..full.len() {
            let _ = parse(&full[..n]);
        }
    }
}
