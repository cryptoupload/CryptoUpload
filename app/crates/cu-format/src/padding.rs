// SPDX-License-Identifier: GPL-3.0-or-later

use crate::content::PLAINTEXT_CHUNK_LEN;
use crate::error::FormatError;

pub const TRAILER_LEN: u64 = 8;

const PT: u64 = PLAINTEXT_CHUNK_LEN as u64;

#[must_use]
pub fn padme(n: u64) -> Option<u64> {
    if n < 2 {
        return Some(n);
    }
    let e = u64::from(n.ilog2());
    let s = u64::from(e.ilog2()) + 1;
    let mask = (1u64 << (e - s)) - 1;
    n.checked_add(mask).map(|v| v & !mask)
}

#[must_use]
pub fn keep_trailer_in_terminal_chunk(t0: u64) -> Option<u64> {
    let r = t0 % PT;
    if r >= TRAILER_LEN {
        Some(t0)
    } else {
        t0.checked_add(TRAILER_LEN - r)
    }
}

#[must_use]
pub fn padded_stream_len(content_len: u64) -> Option<u64> {
    keep_trailer_in_terminal_chunk(padme(content_len.checked_add(TRAILER_LEN)?)?)
}

pub fn content_len(stream_len: u64, terminal_plaintext: &[u8]) -> Result<u64, FormatError> {
    const BAD: FormatError = FormatError::Malformed("file padding not valid");
    let trailer: [u8; 8] = terminal_plaintext
        .len()
        .checked_sub(8)
        .and_then(|start| terminal_plaintext.get(start..))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(BAD)?;
    let declared = u64::from_be_bytes(trailer);
    if declared > stream_len.checked_sub(TRAILER_LEN).ok_or(BAD)? {
        return Err(BAD);
    }
    if padded_stream_len(declared) != Some(stream_len) {
        return Err(BAD);
    }
    Ok(declared)
}
