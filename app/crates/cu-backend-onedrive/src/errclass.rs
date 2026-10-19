// SPDX-License-Identifier: GPL-3.0-or-later

use cu_backend::{BackendError, is_transient_http_status};
use serde_json::Value;

fn graph_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .get("error")?
        .get("code")?
        .as_str()
        .map(str::to_string)
}

fn graph_error_summary(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

pub(crate) fn classify_status(status: reqwest::StatusCode, body: &str) -> BackendError {
    let code = status.as_u16();
    if code == 404 {
        return BackendError::NotFound;
    }
    if code == 409 {
        if graph_error_code(body).as_deref() == Some("nameAlreadyExists") {
            return BackendError::AlreadyExists;
        }
        return BackendError::Other(format!("onedrive: 409: {}", graph_error_summary(body)));
    }
    if code == 507 {
        return BackendError::Coded(
            cu_backend::Msg::new("ONEDRIVE_QUOTA_FULL").arg(graph_error_summary(body)),
        );
    }
    if code == 412 || code == 423 || is_transient_http_status(code) {
        return BackendError::Transient(format!(
            "onedrive: {status}: {}",
            graph_error_summary(body)
        ));
    }
    BackendError::Other(format!("onedrive: {status}: {}", graph_error_summary(body)))
}

pub(crate) fn classify_reqwest(e: &reqwest::Error) -> BackendError {
    cu_backend::http::classify_transport(
        "onedrive",
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

pub(crate) fn foreign_next_link() -> BackendError {
    BackendError::Coded(cu_backend::Msg::new("ONEDRIVE_FOREIGN_NEXT_LINK"))
}
