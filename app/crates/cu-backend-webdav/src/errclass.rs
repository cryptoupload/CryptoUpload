// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::{BackendError, is_transient_http_status};

pub(crate) fn classify_status(status: reqwest::StatusCode, body: &str) -> BackendError {
    match status.as_u16() {
        404 => BackendError::NotFound,
        412 => BackendError::AlreadyExists,
        s if is_transient_http_status(s) => {
            BackendError::Transient(format!("webdav: {status}: {}", truncate(body)))
        }
        _ => BackendError::Other(format!("webdav: {status}: {}", truncate(body))),
    }
}

fn truncate(body: &str) -> &str {
    const MAX: usize = 200;
    if body.len() <= MAX {
        body
    } else {
        let mut end = MAX;
        while end > 0 && !body.is_char_boundary(end) {
            end -= 1;
        }
        &body[..end]
    }
}

pub(crate) fn classify_reqwest(e: &reqwest::Error) -> BackendError {
    cu_backend::http::classify_transport(
        "webdav",
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
