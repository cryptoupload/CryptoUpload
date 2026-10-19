// SPDX-License-Identifier: GPL-3.0-or-later

use async_trait::async_trait;
use cu_backend::{BackendError, StreamReader};
use russh_sftp::client::fs::File;
use tokio::io::AsyncReadExt;

use crate::backend::map_io_err;

pub struct SftpReader {
    file: File,
}

impl SftpReader {
    pub(crate) fn new(file: File) -> Self {
        Self { file }
    }
}

#[async_trait]
impl StreamReader for SftpReader {
    async fn read_chunk(&mut self, max_len: usize) -> Result<Vec<u8>, BackendError> {
        let mut buf = vec![0u8; max_len];
        let mut filled = 0;
        while filled < max_len {
            let n = self
                .file
                .read(&mut buf[filled..])
                .await
                .map_err(map_io_err)?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        buf.truncate(filled);
        Ok(buf)
    }
}
