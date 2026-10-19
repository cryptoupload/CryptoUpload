// SPDX-License-Identifier: GPL-3.0-or-later

use async_trait::async_trait;
use cu_backend::{BackendError, StreamReader};

use crate::errclass::{classify_reqwest, classify_status};

pub struct GDriveReader {
    http: reqwest::Client,
    token: String,
    files_url: String,
    file_id: String,
    response: Option<reqwest::Response>,
    leftover: Vec<u8>,
    started: bool,
}

impl GDriveReader {
    pub(crate) fn new(
        http: reqwest::Client,
        token: String,
        files_url: String,
        file_id: String,
    ) -> Self {
        Self {
            http,
            token,
            files_url,
            file_id,
            response: None,
            leftover: Vec::new(),
            started: false,
        }
    }

    async fn ensure_started(&mut self) -> Result<(), BackendError> {
        if self.started {
            return Ok(());
        }
        self.started = true;
        let url = format!("{}/{}?alt=media", self.files_url, self.file_id);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| classify_reqwest(&e))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(classify_status(status, &text));
        }
        self.response = Some(resp);
        Ok(())
    }
}

#[async_trait]
impl StreamReader for GDriveReader {
    async fn read_chunk(&mut self, max_len: usize) -> Result<Vec<u8>, BackendError> {
        self.ensure_started().await?;
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
