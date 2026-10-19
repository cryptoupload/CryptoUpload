// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::{BackendError, is_transient_http_status};

pub(crate) fn classify_status(status: reqwest::StatusCode, body: &str) -> BackendError {
    if status.as_u16() == 404 {
        return BackendError::NotFound;
    }
    if is_transient_http_status(status.as_u16()) || is_rate_limit_403(status, body) {
        return BackendError::Transient(format!("gdrive: {status}: {body}"));
    }
    BackendError::Other(format!("gdrive: {status}: {body}"))
}

fn is_rate_limit_403(status: reqwest::StatusCode, body: &str) -> bool {
    if status.as_u16() != 403 {
        return false;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    value["error"]["errors"].as_array().is_some_and(|errors| {
        errors.iter().any(|e| {
            matches!(
                e["reason"].as_str(),
                Some("rateLimitExceeded" | "userRateLimitExceeded")
            )
        })
    })
}

pub(crate) fn classify_reqwest(e: &reqwest::Error) -> BackendError {
    cu_backend::http::classify_transport(
        "gdrive",
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
