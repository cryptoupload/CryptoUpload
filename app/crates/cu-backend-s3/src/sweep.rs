// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;

use cu_backend::BackendError;

use crate::backend::S3Backend;
use crate::errclass::classify;

impl S3Backend {
    pub async fn sweep_orphans(&self, cutoff_ms: i64) -> Result<Vec<String>, BackendError> {
        let mut aborted = Vec::new();
        let mut aborted_set: HashSet<String> = HashSet::new();

        for (object_key, upload_id) in self.resume.list_stale(&self.remote_id, cutoff_ms).await? {
            if let Some(upload_id) = &upload_id {
                let _ = self.abort_multipart_upload(&object_key, upload_id).await;
            }
            self.resume.abort(&self.remote_id, &object_key).await?;
            aborted_set.insert(object_key.clone());
            aborted.push(object_key);
        }

        let listing_prefix = if self.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", self.prefix)
        };
        let mut key_marker: Option<String> = None;
        let mut upload_id_marker: Option<String> = None;
        loop {
            let out = self
                .client
                .list_multipart_uploads()
                .bucket(&self.bucket)
                .prefix(&listing_prefix)
                .set_key_marker(key_marker.clone())
                .set_upload_id_marker(upload_id_marker.clone())
                .send()
                .await
                .map_err(|e| classify(&e))?;

            for upload in out.uploads() {
                let (Some(key), Some(upload_id)) = (upload.key(), upload.upload_id()) else {
                    continue;
                };
                if aborted_set.contains(key) {
                    continue;
                }
                if self.resume.load(&self.remote_id, key).await?.is_some() {
                    continue;
                }
                let is_old_enough = upload
                    .initiated()
                    .is_some_and(|t| t.secs().saturating_mul(1000) < cutoff_ms);
                if !is_old_enough {
                    continue;
                }
                let _ = self.abort_multipart_upload(key, upload_id).await;
                aborted_set.insert(key.to_string());
                aborted.push(key.to_string());
            }

            if out.is_truncated().unwrap_or(false) {
                let next_key = out.next_key_marker().map(str::to_string);
                let next_upload = out.next_upload_id_marker().map(str::to_string);
                if next_key.is_none() && next_upload.is_none() {
                    break;
                }
                key_marker = next_key;
                upload_id_marker = next_upload;
            } else {
                break;
            }
        }

        Ok(aborted)
    }

    async fn abort_multipart_upload(&self, key: &str, upload_id: &str) -> Result<(), BackendError> {
        self.client
            .abort_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .upload_id(upload_id)
            .send()
            .await
            .map_err(|e| classify(&e))?;
        Ok(())
    }
}
