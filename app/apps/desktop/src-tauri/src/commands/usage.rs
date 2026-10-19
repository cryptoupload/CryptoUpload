// SPDX-License-Identifier: GPL-3.0-or-later

use tauri::State;

use crate::error::UiError;
use crate::state::AppState;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobUsageDto {
    job_name: String,
    file_count: u64,
    total_bytes: u64,
    version_count: u64,
    version_bytes: u64,
    total_bytes_transferred_lifetime: u64,
}

impl From<cu_core::JobUsage> for JobUsageDto {
    fn from(usage: cu_core::JobUsage) -> Self {
        Self {
            job_name: usage.job_name,
            file_count: usage.file_count,
            total_bytes: usage.total_bytes,
            version_count: usage.version_count,
            version_bytes: usage.version_bytes,
            total_bytes_transferred_lifetime: usage.total_bytes_transferred_lifetime,
        }
    }
}

#[tauri::command]
pub async fn storage_usage(state: State<'_, AppState>) -> Result<Vec<JobUsageDto>, UiError> {
    Ok(cu_core::storage_usage(&state.data_dir)?
        .into_iter()
        .map(Into::into)
        .collect())
}
