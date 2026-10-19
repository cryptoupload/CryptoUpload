// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::{BackendError, is_transient_http_status};
use serde_json::Value;

pub(crate) fn classify_status(status: reqwest::StatusCode, body: &str) -> BackendError {
    if status.as_u16() == 409 {
        return classify_conflict_body(body);
    }
    if is_transient_http_status(status.as_u16()) {
        return BackendError::Transient(format!("dropbox: {status}: {body}"));
    }
    BackendError::Other(format!("dropbox: {status}: {body}"))
}

fn classify_conflict_body(body: &str) -> BackendError {
    let summary = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error_summary")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    if summary.starts_with("path/not_found")
        || summary.starts_with("path_lookup/not_found")
        || summary.starts_with("from_lookup/not_found")
    {
        return BackendError::NotFound;
    }
    if summary.starts_with("path/conflict") || summary.starts_with("to/conflict") {
        return BackendError::AlreadyExists;
    }
    BackendError::Other(format!("dropbox: 409: {summary}"))
}

pub(crate) fn classify_reqwest(e: &reqwest::Error) -> BackendError {
    cu_backend::http::classify_transport(
        "dropbox",
        cu_backend::http::TransportFlags {
            timeout: e.is_timeout(),
            connect: e.is_connect(),
            body: e.is_body(),
            decode: e.is_decode(),
            request: e.is_request(),
        },
    )
}

pub(crate) fn retry_after_ms(resp: &reqwest::Response) -> Option<u64> {
    cu_backend::http::retry_after_ms(
        resp.headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
    )
}
