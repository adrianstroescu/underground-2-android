//! Bounds-checked little-endian readers over byte slices.

use crate::{bail, Result};

#[inline]
fn arr<const N: usize>(b: &[u8], o: usize) -> Result<[u8; N]> {
    match b.get(o..o + N) {
        Some(s) => Ok(s.try_into().unwrap()),
        None => bail!("read of {N} bytes at {o:#x} past end ({:#x})", b.len()),
    }
}

pub fn u8_at(b: &[u8], o: usize) -> Result<u8> {
    Ok(arr::<1>(b, o)?[0])
}

pub fn u16_at(b: &[u8], o: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(arr(b, o)?))
}

pub fn i16_at(b: &[u8], o: usize) -> Result<i16> {
    Ok(i16::from_le_bytes(arr(b, o)?))
}

pub fn u32_at(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(arr(b, o)?))
}

pub fn f32_at(b: &[u8], o: usize) -> Result<f32> {
    Ok(f32::from_le_bytes(arr(b, o)?))
}

pub fn vec3_at(b: &[u8], o: usize) -> Result<[f32; 3]> {
    Ok([f32_at(b, o)?, f32_at(b, o + 4)?, f32_at(b, o + 8)?])
}

pub fn mat4_at(b: &[u8], o: usize) -> Result<[f32; 16]> {
    let mut m = [0f32; 16];
    for (i, v) in m.iter_mut().enumerate() {
        *v = f32_at(b, o + i * 4)?;
    }
    Ok(m)
}

pub fn slice(b: &[u8], o: usize, len: usize) -> Result<&[u8]> {
    match b.get(o..o + len) {
        Some(s) => Ok(s),
        None => bail!("slice {o:#x}+{len:#x} past end ({:#x})", b.len()),
    }
}

/// NUL-terminated fixed-size string field (Latin-1).
pub fn cstr_at(b: &[u8], o: usize, max: usize) -> Result<String> {
    let s = slice(b, o, max.min(b.len().saturating_sub(o)))?;
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    Ok(s[..end].iter().map(|&c| c as char).collect())
}

/// EA pads chunk bodies with 0x11 bytes so that the payload is aligned.
pub fn strip_pad(b: &[u8]) -> &[u8] {
    let mut s = b;
    while s.len() >= 4 && s[..4] == [0x11; 4] {
        s = &s[4..];
    }
    s
}
