// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::Arc;

use async_trait::async_trait;
use cu_backend::{BackendError, StreamWriter};
use russh_sftp::client::SftpSession;
use russh_sftp::client::fs::File;
use tokio::io::AsyncWriteExt;

use crate::backend::{map_io_err, map_sftp_err};

pub struct SftpWriter {
    sftp: Arc<SftpSession>,
    tmp_path: String,
    dest_path: String,
    resume_path: String,
    file: Option<File>,
    exclusive: bool,
}

impl SftpWriter {
    pub(crate) fn new(
        sftp: Arc<SftpSession>,
        tmp_path: String,
        dest_path: String,
        resume_path: String,
        file: File,
        exclusive: bool,
    ) -> Self {
        Self {
            sftp,
            tmp_path,
            dest_path,
            resume_path,
            file: Some(file),
            exclusive,
        }
    }
}

#[async_trait]
impl StreamWriter for SftpWriter {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        self.file
            .as_mut()
            .expect("write_chunk is never called after commit")
            .write_all(data)
            .await
            .map_err(map_io_err)
    }

    async fn commit(mut self: Box<Self>) -> Result<(), BackendError> {
        let file = self
            .file
            .take()
            .expect("commit consumes self: called at most once");
        file.close().await.map_err(map_io_err)?;

        if self.exclusive {
            if let Err(e) = self.sftp.rename(&self.tmp_path, &self.dest_path).await {
                let _ = self.sftp.remove_file(&self.tmp_path).await;
                return if self.sftp.try_exists(&self.dest_path).await.unwrap_or(false) {
                    Err(BackendError::AlreadyExists)
                } else {
                    Err(map_sftp_err(e))
                };
            }
        } else if let Err(first_err) = self.sftp.rename(&self.tmp_path, &self.dest_path).await {
            if !self
                .sftp
                .try_exists(&self.tmp_path)
                .await
                .map_err(map_sftp_err)?
            {
                return Err(map_sftp_err(first_err));
            }
            let _ = self.sftp.remove_file(&self.dest_path).await;
            self.sftp
                .rename(&self.tmp_path, &self.dest_path)
                .await
                .map_err(map_sftp_err)?;
        }
        let _ = self.sftp.remove_file(&self.resume_path).await;
        Ok(())
    }
}

impl Drop for SftpWriter {
    fn drop(&mut self) {
        let Some(file) = self.file.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let sftp = Arc::clone(&self.sftp);
        let tmp = std::mem::take(&mut self.tmp_path);
        runtime.spawn(async move {
            drop(file);
            let _ = sftp.remove_file(&tmp).await;
        });
    }
}
