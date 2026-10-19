// SPDX-License-Identifier: GPL-3.0-or-later

use async_trait::async_trait;
use aws_smithy_types::byte_stream::ByteStream;
use cu_backend::{BackendError, StreamReader};

pub struct S3Reader {
    body: ByteStream,
    leftover: Vec<u8>,
}

impl S3Reader {
    pub const fn new(body: ByteStream) -> Self {
        Self {
            body,
            leftover: Vec::new(),
        }
    }
}

#[async_trait]
impl StreamReader for S3Reader {
    async fn read_chunk(&mut self, max_len: usize) -> Result<Vec<u8>, BackendError> {
        let mut out = Vec::with_capacity(max_len.min(1 << 20));

        if !self.leftover.is_empty() {
            let take = self.leftover.len().min(max_len);
            out.extend_from_slice(&self.leftover[..take]);
            self.leftover.drain(..take);
        }

        while out.len() < max_len {
            match self.body.next().await {
                Some(Ok(bytes)) => {
                    let need = max_len - out.len();
                    if bytes.len() <= need {
                        out.extend_from_slice(&bytes);
                    } else {
                        out.extend_from_slice(&bytes[..need]);
                        self.leftover.extend_from_slice(&bytes[need..]);
                        break;
                    }
                }
                Some(Err(e)) => {
                    return Err(BackendError::Transient(format!(
                        "s3: body read interrupted: {e}"
                    )));
                }
                None => break,
            }
        }

        Ok(out)
    }
}
