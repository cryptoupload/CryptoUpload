// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Clone)]
pub struct GDriveConfig {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
    pub folder_id: String,
}
