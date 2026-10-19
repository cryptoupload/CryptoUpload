// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use cu_backend::BackendError;
use cu_oauth::Endpoints;
use tokio::sync::Mutex;

use crate::config::OneDriveConfig;

pub type RefreshTokenSink = Arc<dyn Fn(String) + Send + Sync>;

pub(crate) struct TokenSource {
    client_id: String,
    client_secret: Option<String>,
    endpoints: Endpoints,
    refresh_token: std::sync::Mutex<String>,
    cached: Mutex<Option<(String, Instant)>>,
    sink: std::sync::Mutex<Option<RefreshTokenSink>>,
}

impl TokenSource {
    pub(crate) fn new(config: &OneDriveConfig) -> Self {
        Self::with_endpoints(config, crate::oauth::tenant_endpoints(&config.tenant))
    }

    pub(crate) fn with_endpoints(config: &OneDriveConfig, endpoints: Endpoints) -> Self {
        Self {
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            endpoints,
            refresh_token: std::sync::Mutex::new(config.refresh_token.clone()),
            cached: Mutex::new(None),
            sink: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn set_sink(&self, sink: RefreshTokenSink) {
        *self.sink.lock().unwrap_or_else(PoisonError::into_inner) = Some(sink);
    }

    pub(crate) async fn get(&self) -> Result<String, BackendError> {
        {
            let guard = self.cached.lock().await;
            if let Some((token, expires_at)) = guard.as_ref()
                && *expires_at > Instant::now() + Duration::from_secs(30)
            {
                return Ok(token.clone());
            }
        }
        self.refresh().await
    }

    pub(crate) async fn refresh(&self) -> Result<String, BackendError> {
        let current = self
            .refresh_token
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let refreshed = crate::oauth::refresh_with_endpoints(
            &self.endpoints,
            &self.client_id,
            self.client_secret.as_deref(),
            &current,
        )
        .await?;
        if let Some(rotated) = refreshed.refresh_token.filter(|t| *t != current) {
            self.refresh_token
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone_from(&rotated);
            let sink = self
                .sink
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(sink) = sink {
                sink(rotated);
            }
        }
        let token = refreshed.access_token;
        let expires_at = Instant::now() + refreshed.expires_in.unwrap_or(Duration::from_secs(1800));
        *self.cached.lock().await = Some((token.clone(), expires_at));
        Ok(token)
    }
}
