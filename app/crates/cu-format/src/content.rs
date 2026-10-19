// SPDX-License-Identifier: GPL-3.0-or-later

use cu_crypto::aead::{ContentKey, NONCE_LEN, PLAINTEXT_CHUNK, TAG_LEN};

use crate::error::FormatError;

pub use cu_crypto::aead::{
    CIPHERTEXT_CHUNK as CIPHERTEXT_CHUNK_LEN, PLAINTEXT_CHUNK as PLAINTEXT_CHUNK_LEN,
};

pub const MIN_CIPHERTEXT_CHUNK: usize = NONCE_LEN + TAG_LEN;

pub struct ContentEncryptor {
    content_key: ContentKey,
    header_nonce: [u8; NONCE_LEN],
    next_index: u64,
    needs_eof_block: bool,
}

impl ContentEncryptor {
    #[must_use]
    pub fn new(content_key: ContentKey, header_nonce: [u8; NONCE_LEN]) -> Self {
        Self {
            content_key,
            header_nonce,
            next_index: 0,
            needs_eof_block: true,
        }
    }

    pub fn seal_next(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, FormatError> {
        let sealed = cu_crypto::aead::seal_chunk(
            &self.content_key,
            self.next_index,
            &self.header_nonce,
            plaintext,
        )?;
        self.next_index += 1;
        self.needs_eof_block = plaintext.len() == PLAINTEXT_CHUNK;
        Ok(sealed)
    }

    pub fn finish(&mut self) -> Result<Option<Vec<u8>>, FormatError> {
        if self.needs_eof_block {
            Ok(Some(self.seal_next(&[])?))
        } else {
            Ok(None)
        }
    }
}

pub struct ContentDecryptor {
    content_key: ContentKey,
    header_nonce: [u8; NONCE_LEN],
    next_index: u64,
    done: bool,
}

impl ContentDecryptor {
    #[must_use]
    pub fn new(content_key: ContentKey, header_nonce: [u8; NONCE_LEN]) -> Self {
        Self {
            content_key,
            header_nonce,
            next_index: 0,
            done: false,
        }
    }

    pub fn feed_chunk(&mut self, ciphertext_chunk: &[u8]) -> Result<Vec<u8>, FormatError> {
        if self.done {
            return Err(FormatError::DecryptFailed);
        }
        let plaintext = cu_crypto::aead::open_chunk(
            &self.content_key,
            self.next_index,
            &self.header_nonce,
            ciphertext_chunk,
        )?;
        self.next_index += 1;
        if plaintext.len() < PLAINTEXT_CHUNK {
            self.done = true;
        }
        Ok(plaintext)
    }

    #[must_use]
    pub const fn is_done(&self) -> bool {
        self.done
    }
}
