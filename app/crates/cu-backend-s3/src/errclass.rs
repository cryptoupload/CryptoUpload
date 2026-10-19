// SPDX-License-Identifier: GPL-3.0-or-later

use aws_sdk_s3::error::{ProvideErrorMetadata, SdkError};
use cu_backend::{BackendError, is_transient_http_status};

const TRANSIENT_CODES: &[&str] = &[
    "SlowDown",
    "RequestTimeout",
    "RequestTimeTooSkewed",
    "InternalError",
    "ServiceUnavailable",
    "Throttling",
    "ThrottlingException",
];

fn is_object_lock_denial(code: Option<&str>, message: Option<&str>) -> bool {
    if !matches!(code, Some("AccessDenied" | "InvalidRequest")) {
        return false;
    }
    message.is_some_and(|m| {
        let lower = m.to_ascii_lowercase();
        lower.contains("object lock") || lower.contains("worm")
    })
}

pub fn classify<E>(err: &SdkError<E>) -> BackendError
where
    E: ProvideErrorMetadata + std::fmt::Debug,
{
    match err {
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_) => {
            return BackendError::Transient(format!("{err}"));
        }
        SdkError::ConstructionFailure(_) => {
            return BackendError::Other(format!("{err}"));
        }
        _ => {}
    }

    let status = err.raw_response().map(|r| r.status().as_u16());
    let code = err.code().map(str::to_string);
    if matches!(status, Some(412 | 409)) {
        return BackendError::AlreadyExists;
    }
    if is_object_lock_denial(code.as_deref(), err.message()) {
        return BackendError::RetentionActive(
            err.message()
                .map_or_else(|| format!("{err}"), str::to_string),
        );
    }
    let transient = status.is_some_and(is_transient_http_status)
        || code
            .as_deref()
            .is_some_and(|c| TRANSIENT_CODES.contains(&c));

    let msg = code.map_or_else(|| format!("{err}"), |c| format!("{c}: {err}"));
    if transient {
        BackendError::Transient(msg)
    } else {
        BackendError::Other(msg)
    }
}
