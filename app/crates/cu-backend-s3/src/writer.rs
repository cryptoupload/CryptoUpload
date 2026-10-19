// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use aws_sdk_s3::Client;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use bytes::Bytes;
use cu_backend::resume::{PartRecord, ResumeStore};
use cu_backend::{BackendError, CircuitBreaker, RetryPolicy, StreamWriter, SystemClock, retry};

use crate::config::{ObjectLockMode, retain_until_date};
use crate::errclass::classify;

pub const PART_SIZE: usize = 8 * 1024 * 1024;

const MAX_PART_NUMBER: u32 = 10_000;

fn part_number_i32(part_number: u32) -> Result<i32, BackendError> {
    i32::try_from(part_number)
        .map_err(|_| BackendError::Other(format!("s3: part number {part_number} out of range")))
}

pub(crate) fn fresh_source_marker() -> [u8; 32] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(&nanos.to_le_bytes());
    out[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    out
}

async fn sleep_ms(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

pub struct S3Writer {
    client: Client,
    bucket: String,
    key: String,
    remote_id: String,
    resume: Arc<dyn ResumeStore>,
    retry_policy: RetryPolicy,
    breaker: Arc<CircuitBreaker>,

    buf: Vec<u8>,
    part_number: u32,
    upload_id: Option<String>,
    completed: Vec<CompletedPart>,
    known_parts: HashMap<u32, PartRecord>,
    row_exists: bool,
    resume_checked: bool,
    exclusive: bool,
    object_lock_mode: Option<ObjectLockMode>,
    object_lock_days: Option<u16>,
}

impl S3Writer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: Client,
        bucket: String,
        key: String,
        remote_id: String,
        resume: Arc<dyn ResumeStore>,
        retry_policy: RetryPolicy,
        breaker: Arc<CircuitBreaker>,
        exclusive: bool,
        object_lock_mode: Option<ObjectLockMode>,
        object_lock_days: Option<u16>,
    ) -> Self {
        Self {
            client,
            bucket,
            key,
            remote_id,
            resume,
            retry_policy,
            breaker,
            buf: Vec::new(),
            part_number: 0,
            upload_id: None,
            completed: Vec::new(),
            known_parts: HashMap::new(),
            row_exists: false,
            resume_checked: false,
            exclusive,
            object_lock_mode,
            object_lock_days,
        }
    }

    fn seed(&self) -> u64 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
        nanos ^ u64::from(self.part_number)
    }

    async fn create_multipart_upload(&self) -> Result<String, BackendError> {
        let out = retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            self.seed(),
            sleep_ms,
            |_attempt| async {
                let mut req = self
                    .client
                    .create_multipart_upload()
                    .bucket(&self.bucket)
                    .key(&self.key);
                if let Some(mode) = self.object_lock_mode {
                    req = req.object_lock_mode(mode.to_sdk());
                    if let Some(until) = retain_until_date(self.object_lock_days) {
                        req = req.object_lock_retain_until_date(until);
                    }
                }
                req.send().await.map_err(|e| classify(&e))
            },
        )
        .await?;
        out.upload_id().map(str::to_string).ok_or_else(|| {
            BackendError::Other("s3: CreateMultipartUpload without UploadId".to_string())
        })
    }

    async fn upload_part(
        &self,
        upload_id: &str,
        part_number: u32,
        owned: Bytes,
    ) -> Result<String, BackendError> {
        let out = retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            self.seed(),
            sleep_ms,
            |_attempt| async {
                self.client
                    .upload_part()
                    .bucket(&self.bucket)
                    .key(&self.key)
                    .upload_id(upload_id)
                    .part_number(part_number_i32(part_number)?)
                    .body(ByteStream::from(owned.clone()))
                    .send()
                    .await
                    .map_err(|e| classify(&e))
            },
        )
        .await?;
        out.e_tag()
            .map(str::to_string)
            .ok_or_else(|| BackendError::Other("s3: UploadPart without ETag".to_string()))
    }

    async fn put_object(&self, bytes: Vec<u8>) -> Result<(), BackendError> {
        let owned = Bytes::from(bytes);
        retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            self.seed(),
            sleep_ms,
            |_attempt| async {
                let mut req = self
                    .client
                    .put_object()
                    .bucket(&self.bucket)
                    .key(&self.key)
                    .body(ByteStream::from(owned.clone()));
                if self.exclusive {
                    req = req.if_none_match("*");
                }
                if let Some(mode) = self.object_lock_mode {
                    req = req.object_lock_mode(mode.to_sdk());
                    if let Some(until) = retain_until_date(self.object_lock_days) {
                        req = req.object_lock_retain_until_date(until);
                    }
                }
                req.send().await.map_err(|e| classify(&e))
            },
        )
        .await?;
        Ok(())
    }

    async fn complete_multipart_upload(
        &self,
        upload_id: &str,
        parts: CompletedMultipartUpload,
    ) -> Result<(), BackendError> {
        retry(
            &self.retry_policy,
            &self.breaker,
            &SystemClock,
            self.seed(),
            sleep_ms,
            |_attempt| async {
                let mut req = self
                    .client
                    .complete_multipart_upload()
                    .bucket(&self.bucket)
                    .key(&self.key)
                    .upload_id(upload_id)
                    .multipart_upload(parts.clone());
                if self.exclusive {
                    req = req.if_none_match("*");
                }
                req.send().await.map_err(|e| classify(&e))
            },
        )
        .await?;
        Ok(())
    }

    async fn process_part(&mut self, bytes: Vec<u8>) -> Result<(), BackendError> {
        self.part_number += 1;
        if self.part_number > MAX_PART_NUMBER {
            return Err(BackendError::Other(format!(
                "s3: limit of {MAX_PART_NUMBER} parts exceeded for {}",
                self.key
            )));
        }

        if !self.resume_checked {
            self.resume_checked = true;
            if let Some(record) = self.resume.load(&self.remote_id, &self.key).await? {
                self.row_exists = true;
                self.known_parts = record
                    .parts
                    .into_iter()
                    .map(|p| (p.part_number, p))
                    .collect();
                self.upload_id = record.upload_id;
            }
        }

        let len = bytes.len() as u64;

        if let Some(known) = self.known_parts.get(&self.part_number)
            && known.len == len
        {
            self.completed.push(
                CompletedPart::builder()
                    .part_number(part_number_i32(self.part_number)?)
                    .e_tag(known.etag.clone())
                    .build(),
            );
            return Ok(());
        }

        let crc = crc32c::crc32c(&bytes).to_be_bytes();

        if self.upload_id.is_none() {
            if !self.row_exists {
                self.resume
                    .begin(
                        &self.remote_id,
                        &self.key,
                        PART_SIZE as u64,
                        fresh_source_marker(),
                    )
                    .await?;
                self.row_exists = true;
            }
            let id = self.create_multipart_upload().await?;
            self.resume
                .set_upload_id(&self.remote_id, &self.key, &id)
                .await?;
            self.upload_id = Some(id);
        }

        let upload_id = self.upload_id.clone().expect("assigned just above");
        let etag = self
            .upload_part(&upload_id, self.part_number, Bytes::from(bytes))
            .await?;

        self.resume
            .record_part(
                &self.remote_id,
                &self.key,
                PartRecord {
                    part_number: self.part_number,
                    etag: etag.clone(),
                    crc32c: crc,
                    len,
                },
            )
            .await?;

        self.completed.push(
            CompletedPart::builder()
                .part_number(part_number_i32(self.part_number)?)
                .e_tag(etag)
                .build(),
        );
        Ok(())
    }
}

#[async_trait]
impl StreamWriter for S3Writer {
    async fn write_chunk(&mut self, data: &[u8]) -> Result<(), BackendError> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= PART_SIZE {
            let part = std::mem::replace(&mut self.buf, Vec::with_capacity(PART_SIZE + data.len()));
            self.process_part(part).await?;
        }
        Ok(())
    }

    async fn commit(mut self: Box<Self>) -> Result<(), BackendError> {
        if self.part_number == 0 {
            let body = std::mem::take(&mut self.buf);
            self.put_object(body).await?;
            self.resume.finish(&self.remote_id, &self.key).await?;
            return Ok(());
        }

        if !self.buf.is_empty() {
            let last = std::mem::take(&mut self.buf);
            self.process_part(last).await?;
        }

        let upload_id = self.upload_id.clone().expect("multipart already started");
        let mut parts = std::mem::take(&mut self.completed);
        parts.sort_by_key(CompletedPart::part_number);
        let completed_upload = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();

        self.complete_multipart_upload(&upload_id, completed_upload)
            .await?;
        self.resume.finish(&self.remote_id, &self.key).await?;
        Ok(())
    }
}
