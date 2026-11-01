//! A bounds-checked little-endian cursor: for retail records, and the
//! game's own navmesh cache.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("record ends early: needed {needed} bytes at offset {offset}, {available} available")]
pub struct Truncated {
    pub offset: usize,
    pub needed: usize,
    pub available: usize,
}

/// A record that could not be decoded.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error(transparent)]
    Truncated(#[from] Truncated),
    #[error("{0} at offset {1}")]
    Invalid(&'static str, usize),
}

pub struct Bytes<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Bytes { data, offset: 0 }
    }

    /// A cursor over `data` from `offset`, for fields at fixed offsets.
    pub fn at(data: &'a [u8], offset: usize) -> Result<Self, Truncated> {
        if offset > data.len() {
            return Err(Truncated { offset: 0, needed: offset, available: data.len() });
        }
        Ok(Bytes { data, offset })
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The record is invalid at the current offset.
    pub fn invalid(&self, message: &'static str) -> DecodeError {
        DecodeError::Invalid(message, self.offset)
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.offset
    }

    pub fn take(&mut self, count: usize) -> Result<&'a [u8], Truncated> {
        if count > self.remaining() {
            return Err(Truncated {
                offset: self.offset,
                needed: count,
                available: self.remaining(),
            });
        }
        let slice = &self.data[self.offset..self.offset + count];
        self.offset += count;
        Ok(slice)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], Truncated> {
        Ok(self.take(N)?.try_into().unwrap())
    }

    pub fn u8(&mut self) -> Result<u8, Truncated> {
        Ok(self.array::<1>()?[0])
    }

    pub fn i8(&mut self) -> Result<i8, Truncated> {
        Ok(self.u8()? as i8)
    }

    pub fn u16(&mut self) -> Result<u16, Truncated> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn i16(&mut self) -> Result<i16, Truncated> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32, Truncated> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32, Truncated> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> Result<f32, Truncated> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    pub fn f32s<const N: usize>(&mut self) -> Result<[f32; N], Truncated> {
        let mut out = [0.0; N];
        for value in &mut out {
            *value = self.f32()?;
        }
        Ok(out)
    }

    pub fn i32s<const N: usize>(&mut self) -> Result<[i32; N], Truncated> {
        let mut out = [0; N];
        for value in &mut out {
            *value = self.i32()?;
        }
        Ok(out)
    }

    /// Read `count` records, refusing counts the remaining bytes cannot hold.
    pub fn records<T>(
        &mut self,
        count: usize,
        record_size: usize,
        mut read: impl FnMut(&mut Self) -> Result<T, Truncated>,
    ) -> Result<Vec<T>, Truncated> {
        let needed = count * record_size;
        if needed > self.remaining() {
            return Err(Truncated { offset: self.offset, needed, available: self.remaining() });
        }
        (0..count).map(|_| read(self)).collect()
    }
}

/// A little-endian `u16` at `at` in `data`.
pub fn u16_at(data: &[u8], at: usize) -> Result<u16, Truncated> {
    Bytes::at(data, at)?.u16()
}

/// A little-endian `i16` at `at` in `data`.
pub fn i16_at(data: &[u8], at: usize) -> Result<i16, Truncated> {
    Bytes::at(data, at)?.i16()
}

/// A little-endian `u32` at `at` in `data`.
pub fn u32_at(data: &[u8], at: usize) -> Result<u32, Truncated> {
    Bytes::at(data, at)?.u32()
}

/// A little-endian `i32` at `at` in `data`.
pub fn i32_at(data: &[u8], at: usize) -> Result<i32, Truncated> {
    Bytes::at(data, at)?.i32()
}

/// A little-endian `f32` at `at` in `data`.
pub fn f32_at(data: &[u8], at: usize) -> Result<f32, Truncated> {
    Bytes::at(data, at)?.f32()
}

/// A UTF-16LE field's code units up to its first NUL (or its whole length).
pub fn utf16_units(field: &[u8]) -> Vec<u16> {
    field
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .take_while(|&u| u != 0)
        .collect()
}

/// A UTF-16LE text field up to its first NUL, invalid units replaced.
pub fn utf16_string(field: &[u8]) -> String {
    String::from_utf16_lossy(&utf16_units(field))
}

/// A fixed-size text field up to its first NUL (or its whole length).
pub fn lenient_string(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// A fixed-size, NUL-terminated text field; `None` if unterminated.
pub fn c_string(field: &[u8]) -> Option<String> {
    let end = field.iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&field[..end]).into_owned())
}
