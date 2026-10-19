// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::Secret;
use cu_crypto::seed::RecoveryPhrase;
use cu_crypto::slip39;
use cu_msg::{Coded, Msg};
use zeroize::Zeroizing;

use crate::error::CoreError;

const PHRASE_WORDS: usize = 24;

fn shares_error(e: &slip39::Slip39Error) -> CoreError {
    CoreError::RecoveryShares(e.msg())
}

pub fn split_recovery_phrase(
    phrase: &RecoveryPhrase,
    threshold: u8,
    count: u8,
) -> Result<Vec<Zeroizing<String>>, CoreError> {
    if threshold < 2 {
        return Err(CoreError::RecoveryShares(Msg::new("SHARES_THRESHOLD_MIN")));
    }
    let entropy = phrase.entropy();
    slip39::split(entropy.expose(), threshold, count, "").map_err(|e| shares_error(&e))
}

pub fn recovery_entropy_from_text(text: &str) -> Result<Secret<32>, CoreError> {
    let pieces: Vec<&str> = text
        .split(['\n', ';'])
        .map(str::trim)
        .filter(|piece| !piece.is_empty())
        .collect();
    let total_words: usize = pieces.iter().map(|p| p.split_whitespace().count()).sum();
    if total_words == PHRASE_WORDS {
        let phrase = RecoveryPhrase::parse(&pieces.join(" "))
            .map_err(cu_format::error::FormatError::from)?;
        return Ok(phrase.entropy());
    }
    if pieces.is_empty() {
        return Err(CoreError::RecoveryShares(Msg::new("SHARES_EMPTY")));
    }
    let secret = slip39::combine(&pieces, "").map_err(|e| shares_error(&e))?;
    let bytes: [u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| CoreError::RecoveryShares(Msg::new("SHARES_NOT_OURS")))?;
    Ok(Secret::new(bytes))
}
