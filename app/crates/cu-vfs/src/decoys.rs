// SPDX-License-Identifier: GPL-3.0-or-later

use cu_format::content::{
    CIPHERTEXT_CHUNK_LEN as CT, MIN_CIPHERTEXT_CHUNK, PLAINTEXT_CHUNK_LEN as PT,
};
use cu_format::error::FormatError;
use cu_format::header::HEADER_LEN;
use cu_format::name::{DECOY_MARKER, MAX_PADDED_NAME_BYTES, NAME_PAD_BLOCK};
use cu_format::padding::{TRAILER_LEN, padded_stream_len};

use crate::{RealFile, VfsError};

const MIN_ENTRIES: usize = 4;

pub const MAX_DECOY_STREAM_LEN: u64 = 16 * 1024 * 1024;

const MAX_FALLBACK_LEN_LOG2: u32 = 20;

pub fn wanted(real_files: usize) -> usize {
    real_files
        .checked_next_power_of_two()
        .map_or(0, |n| n.max(MIN_ENTRIES) - real_files)
}

fn random_u64() -> Option<u64> {
    let mut bytes = [0u8; 8];
    cu_crypto::random::fill_random(&mut bytes).ok()?;
    Some(u64::from_le_bytes(bytes))
}

pub fn pick(len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    usize::try_from(random_u64()? % len as u64).ok()
}

fn stream_len(stored_len: u64) -> Option<u64> {
    let body = stored_len.checked_sub(HEADER_LEN as u64)?;
    let tail = (body % CT as u64).checked_sub(MIN_CIPHERTEXT_CHUNK as u64)?;
    Some(body / CT as u64 * PT as u64 + tail)
}

pub fn shape(real_files: &[RealFile]) -> Result<(String, u64), VfsError> {
    let unavailable = || VfsError::Format(FormatError::RandomUnavailable);

    let band = pick(real_files.len())
        .and_then(|i| real_files.get(i))
        .map_or(NAME_PAD_BLOCK, |file| {
            file.name_len
                .next_multiple_of(NAME_PAD_BLOCK)
                .clamp(NAME_PAD_BLOCK, MAX_PADDED_NAME_BYTES)
        });
    let mut random = vec![0u8; band / 2];
    cu_crypto::random::fill_random(&mut random).map_err(|_| unavailable())?;
    let mut name = String::with_capacity(band);
    name.push(DECOY_MARKER);
    for byte in random {
        use std::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    name.truncate(band - 1);

    let imitable: Vec<u64> = real_files
        .iter()
        .filter_map(|file| stream_len(file.stored_len?))
        .filter(|&len| (TRAILER_LEN..=MAX_DECOY_STREAM_LEN).contains(&len))
        .collect();
    let content_len = match pick(imitable.len()).and_then(|i| imitable.get(i)) {
        Some(&stream) if padded_stream_len(stream - TRAILER_LEN) == Some(stream) => {
            stream - TRAILER_LEN
        }
        _ => {
            let bits = random_u64().ok_or_else(unavailable)?;
            let exponent = u32::try_from(bits % u64::from(MAX_FALLBACK_LEN_LOG2 + 1)).unwrap_or(0);
            let base = 1u64 << exponent;
            (base + (bits >> 8) % base).min(1 << MAX_FALLBACK_LEN_LOG2)
        }
    };
    Ok((name, content_len))
}
