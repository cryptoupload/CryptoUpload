// SPDX-License-Identifier: GPL-3.0-or-later

use crate::BackendError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TransportFlags {
    pub timeout: bool,
    pub connect: bool,
    pub body: bool,
    pub decode: bool,
    pub request: bool,
}

#[must_use]
pub fn classify_transport(provider: &str, flags: TransportFlags) -> BackendError {
    let kind = if flags.timeout {
        "timeout"
    } else if flags.connect {
        "connection failed"
    } else if flags.body {
        "error in the request or response body"
    } else if flags.decode {
        "undecodable response"
    } else if flags.request {
        "invalid request"
    } else {
        "transport error"
    };
    let msg = format!("{provider}: {kind}");
    if flags.timeout || flags.connect {
        BackendError::Transient(msg)
    } else {
        BackendError::Other(msg)
    }
}

#[must_use]
pub fn retry_after_ms(header_value: Option<&str>) -> Option<u64> {
    header_value
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|secs| secs.saturating_mul(1000))
}
