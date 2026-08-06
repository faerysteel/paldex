use crate::GvasError;

/// A bounds-checked little-endian byte reader.
///
/// Every read is checked against the slice it was built from — never the whole
/// original file — so a bounded [`Cursor::sub_cursor`] gives a hard, cheap-to-enforce
/// ceiling on how much a nested (and possibly misunderstood) structure can consume.
#[derive(Debug, Clone, Copy)]
pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[must_use]
    pub fn pos(&self) -> usize {
        self.pos
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Jump to an absolute position within this cursor's slice.
    ///
    /// Used to unconditionally resync after decoding a property's value: every
    /// property tag declares its own byte length, so the caller always knows
    /// where the *next* tag starts even if the value's internal structure
    /// wasn't fully understood.
    pub fn set_pos(&mut self, pos: usize) -> Result<(), GvasError> {
        if pos > self.data.len() {
            return Err(GvasError::UnexpectedEof {
                offset: self.pos,
                needed: pos - self.pos,
                available: self.data.len().saturating_sub(self.pos),
            });
        }
        self.pos = pos;
        Ok(())
    }

    /// An independent cursor over the next `len` bytes, without advancing `self`.
    ///
    /// Bounding recursive parsing to a value's declared size means a confused or
    /// wrong nested decoder can run out of bytes to misinterpret quickly, rather
    /// than reading garbage lengths deep into the rest of the file.
    pub fn sub_cursor(&self, len: usize) -> Result<Cursor<'a>, GvasError> {
        let end = self.pos.checked_add(len).filter(|&e| e <= self.data.len());
        let Some(end) = end else {
            return Err(GvasError::UnexpectedEof {
                offset: self.pos,
                needed: len,
                available: self.data.len().saturating_sub(self.pos),
            });
        };
        Ok(Cursor {
            data: &self.data[self.pos..end],
            pos: 0,
        })
    }

    /// This cursor's whole slice, ignoring the current position. Used to capture
    /// a bounded region as opaque bytes when its structure isn't decoded.
    #[must_use]
    pub fn all(&self) -> &'a [u8] {
        self.data
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], GvasError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let Some(end) = end else {
            return Err(GvasError::UnexpectedEof {
                offset: self.pos,
                needed: n,
                available: self.data.len().saturating_sub(self.pos),
            });
        };
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], GvasError> {
        self.take(n)
    }

    pub fn u8(&mut self) -> Result<u8, GvasError> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool, GvasError> {
        Ok(self.u8()? != 0)
    }

    pub fn u16(&mut self) -> Result<u16, GvasError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, GvasError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, GvasError> {
        Ok(self.u32()? as i32)
    }

    pub fn u64(&mut self) -> Result<u64, GvasError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    pub fn i64(&mut self) -> Result<i64, GvasError> {
        Ok(self.u64()? as i64)
    }

    pub fn f32(&mut self) -> Result<f32, GvasError> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn f64(&mut self) -> Result<f64, GvasError> {
        Ok(f64::from_bits(self.u64()?))
    }

    pub fn guid(&mut self) -> Result<[u8; 16], GvasError> {
        let b = self.take(16)?;
        Ok(b.try_into().unwrap_or_else(|_| unreachable!("take(16) is exactly 16 bytes")))
    }

    /// An Unreal `FString`: an `i32` length prefix, positive for an ASCII/UTF-8
    /// byte string (length includes the trailing NUL), negative for a UTF-16LE
    /// string (length in UTF-16 code units, also NUL-terminated), zero for empty.
    pub fn fstring(&mut self) -> Result<String, GvasError> {
        let n = self.i32()?;
        if n == 0 {
            return Ok(String::new());
        }
        if n > 0 {
            #[allow(clippy::cast_sign_loss)]
            let n = n as usize;
            let bytes = self.take(n)?;
            let content = &bytes[..bytes.len().saturating_sub(1)];
            Ok(String::from_utf8_lossy(content).into_owned())
        } else {
            let Some(units) = n.checked_neg() else {
                return Err(GvasError::InvalidStringLength(n));
            };
            #[allow(clippy::cast_sign_loss)]
            let units = units as usize;
            let bytes = self.take(units * 2)?;
            let content = &bytes[..bytes.len().saturating_sub(2)];
            let code_units: Vec<u16> = content
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Ok(String::from_utf16_lossy(&code_units))
        }
    }
}
