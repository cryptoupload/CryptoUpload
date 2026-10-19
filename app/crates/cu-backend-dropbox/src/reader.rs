// SPDX-License-Identifier: GPL-3.0-or-later

use async_trait::async_trait;
use cu_backend::{BackendError, StreamReader};

use crate::errclass::classify_reqwest;

pub struct DropboxReader {
    response: Option<reqwest::Response>,
    leftover: Vec<u8>,
}

impl DropboxReader {
    pub(crate) fn new(response: reqwest::Response) -> Self {
        Self {
            response: Some(response),
            leftover: Vec::new(),
        }
    }
}

#[async_trait]
impl StreamReader for DropboxReader {
    async fn read_chunk(&mut self, max_len: usize) -> Result<Vec<u8>, BackendError> {
        while self.leftover.len() < max_len {
            let Some(resp) = self.response.as_mut() else {
                break;
            };
            if let Some(bytes) = resp.chunk().await.map_err(|e| classify_reqwest(&e))? {
                self.leftover.extend_from_slice(&bytes);
            } else {
                self.response = None;
                break;
            }
        }
        let take = self.leftover.len().min(max_len);
        Ok(self.leftover.drain(..take).collect())
    }
}
