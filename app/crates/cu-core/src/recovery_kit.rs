// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cu_jobs::{JobSpec, VaultRef};

use crate::remote::{OneDriveRootKind, RemoteConfig, S3ObjectLockMode, SftpAuth};
use crate::unlock_methods::UnlockMethod;
use crate::vault::Vault;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultMode {
    LocalOnly,
    LocalPlusSync { remotes: Vec<String> },
    CloudOnly { remote: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobSummary {
    pub name: String,
    pub local_root: String,
    pub vault_path: String,
    pub remote_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSummary {
    pub name: String,
    pub provider: String,
    pub details: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteInfo {
    Known(RemoteSummary),
    Locked { name: String },
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryKit {
    pub app_version: String,
    pub file_format: String,
    pub name_format: String,
    pub kdf_name: String,
    pub mode: VaultMode,
    pub local_path: Option<String>,
    pub has_password: bool,
    pub has_recovery_phrase: bool,
    pub has_fido2: bool,
    pub has_platform_unlock: bool,
    pub members: Vec<String>,
    pub inbox_active: bool,
    pub jobs: Vec<JobSummary>,
    pub remotes: Vec<RemoteInfo>,
}

fn canonical_or(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn vault_ref_matches(a: &VaultRef, b: &VaultRef) -> bool {
    match (a, b) {
        (VaultRef::Local(pa), VaultRef::Local(pb)) => canonical_or(pa) == canonical_or(pb),
        (VaultRef::Remote(na), VaultRef::Remote(nb)) => na == nb,
        _ => false,
    }
}

fn classify_jobs(location: &VaultRef, jobs: &[JobSpec]) -> (Vec<JobSummary>, Vec<String>) {
    let local_root_canon = match location {
        VaultRef::Local(p) => Some(canonical_or(p)),
        VaultRef::Remote(_) => None,
    };

    let mut summaries = Vec::new();
    let mut mirror_remotes = Vec::new();

    for job in jobs {
        let matches_location = vault_ref_matches(&job.vault, location);

        let mut is_mirror = false;
        if let (Some(root), VaultRef::Remote(name)) = (&local_root_canon, &job.vault)
            && job.vault_path.is_empty()
            && canonical_or(&job.local_root) == *root
        {
            is_mirror = true;
            mirror_remotes.push(name.clone());
        }

        if matches_location || is_mirror {
            let remote_name = match &job.vault {
                VaultRef::Remote(n) => Some(n.clone()),
                VaultRef::Local(_) => None,
            };
            summaries.push(JobSummary {
                name: job.name.clone(),
                local_root: job.local_root.display().to_string(),
                vault_path: job.vault_path.clone(),
                remote_name,
            });
        }
    }

    mirror_remotes.sort_unstable();
    mirror_remotes.dedup();
    (summaries, mirror_remotes)
}

#[must_use]
pub fn referenced_remote_names(location: &VaultRef, jobs: &[JobSpec]) -> Vec<String> {
    let (summaries, mirror_remotes) = classify_jobs(location, jobs);
    let mut names = mirror_remotes;
    if let VaultRef::Remote(name) = location {
        names.push(name.clone());
    }
    for job in &summaries {
        if let Some(name) = &job.remote_name {
            names.push(name.clone());
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn summarize_remote(name: &str, cfg: &RemoteConfig, t: &KitText) -> RemoteSummary {
    let pair = |key: &str, value: String| (t.get(key), value);
    let yes_no = |b: bool| t.get(if b { "yes" } else { "no" });
    let (provider, details): (String, Vec<(String, String)>) = match cfg {
        RemoteConfig::S3(c) => (
            t.get("s3Provider"),
            vec![
                pair("endpoint", c.endpoint.clone()),
                pair("region", c.region.clone()),
                pair("bucket", c.bucket.clone()),
                pair(
                    "prefix",
                    if c.prefix.is_empty() {
                        t.get("prefixRoot")
                    } else {
                        c.prefix.clone()
                    },
                ),
                pair(
                    "addressing",
                    t.get(if c.path_style {
                        "pathStyle"
                    } else {
                        "virtualHosted"
                    }),
                ),
                pair("objectLock", {
                    let days = c
                        .object_lock_days
                        .map_or_else(|| "?".to_string(), |d| d.to_string());
                    match c.object_lock_mode {
                        Some(S3ObjectLockMode::Governance) => {
                            t.fill("lockGovernance", &[("days", &days)])
                        }
                        Some(S3ObjectLockMode::Compliance) => {
                            t.fill("lockCompliance", &[("days", &days)])
                        }
                        None => t.get("lockOff"),
                    }
                }),
            ],
        ),
        RemoteConfig::Sftp(c) => (
            "SFTP".to_string(),
            vec![
                pair("host", c.host.clone()),
                pair("port", c.port.to_string()),
                pair("user", c.username.clone()),
                pair(
                    "auth",
                    match &c.auth {
                        SftpAuth::Password { .. } => t.get("authPassword"),
                        SftpAuth::PrivateKey { path, .. } => t.fill("authKey", &[("path", path)]),
                    },
                ),
                pair(
                    "remoteRoot",
                    if c.remote_root.is_empty() {
                        t.get("homeDir")
                    } else {
                        c.remote_root.clone()
                    },
                ),
                pair("hostKeyType", c.host_key_type.clone()),
                pair("hostKeyFingerprint", c.host_key_fingerprint.clone()),
            ],
        ),
        RemoteConfig::GDrive(c) => (
            "Google Drive".to_string(),
            vec![pair("gdriveFolder", c.folder_id.clone())],
        ),
        RemoteConfig::Dropbox(c) => (
            "Dropbox".to_string(),
            vec![pair("oauthClientId", c.client_id.clone())],
        ),
        RemoteConfig::OneDrive(c) => (
            "OneDrive".to_string(),
            vec![
                pair("oauthClientId", c.client_id.clone()),
                pair("tenant", c.tenant.clone()),
                pair("clientSecretSet", yes_no(c.client_secret.is_some())),
                pair(
                    "oneRoot",
                    t.get(match c.root_kind {
                        OneDriveRootKind::AppFolder => "rootAppFolder",
                        OneDriveRootKind::DriveRoot => "rootDrive",
                    }),
                ),
            ],
        ),
        RemoteConfig::WebDav(c) => (
            "WebDAV".to_string(),
            vec![
                pair("webdavUrl", c.base_url.clone()),
                pair("user", c.username.clone()),
                pair(
                    "webdavRoot",
                    if c.root_path.is_empty() {
                        t.get("rootPlain")
                    } else {
                        c.root_path.clone()
                    },
                ),
                pair("insecureHttp", yes_no(c.allow_insecure_http)),
            ],
        ),
    };
    RemoteSummary {
        name: name.to_string(),
        provider,
        details,
    }
}

#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn build(
    vault: &Vault,
    location: &VaultRef,
    jobs: &[JobSpec],
    remote_details: &HashMap<String, RemoteConfig>,
    app_version: &str,
    t: &KitText,
) -> RecoveryKit {
    let local_path = match location {
        VaultRef::Local(p) => Some(p.display().to_string()),
        VaultRef::Remote(_) => None,
    };

    let (jobs_summary, mirror_remotes) = classify_jobs(location, jobs);

    let mode = match location {
        VaultRef::Local(_) if mirror_remotes.is_empty() => VaultMode::LocalOnly,
        VaultRef::Local(_) => VaultMode::LocalPlusSync {
            remotes: mirror_remotes,
        },
        VaultRef::Remote(name) => VaultMode::CloudOnly {
            remote: name.clone(),
        },
    };

    let remote_names = referenced_remote_names(location, jobs);
    let remotes = remote_names
        .into_iter()
        .map(|name| match remote_details.get(&name) {
            Some(cfg) => RemoteInfo::Known(summarize_remote(&name, cfg, t)),
            None => RemoteInfo::Locked { name },
        })
        .collect();

    let unlock_methods = vault.unlock_methods();
    RecoveryKit {
        app_version: app_version.to_string(),
        file_format: vault.file_format().to_string(),
        name_format: vault.name_format().to_string(),
        kdf_name: vault.kdf_name().to_string(),
        mode,
        local_path,
        has_password: unlock_methods
            .iter()
            .any(|m| matches!(m, UnlockMethod::Password)),
        has_recovery_phrase: vault.has_recovery_phrase(),
        has_fido2: unlock_methods
            .iter()
            .any(|m| matches!(m, UnlockMethod::Fido2 { .. })),
        has_platform_unlock: unlock_methods
            .iter()
            .any(|m| matches!(m, UnlockMethod::Platform)),
        members: vault.list_members().into_iter().map(|m| m.name).collect(),
        inbox_active: vault.has_inbox(),
        jobs: jobs_summary,
        remotes,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Debug, Clone, Default)]
pub struct KitText(HashMap<String, String>);

impl KitText {
    #[must_use]
    pub const fn new(texts: HashMap<String, String>) -> Self {
        Self(texts)
    }

    #[must_use]
    pub fn get(&self, key: &str) -> String {
        self.0.get(key).cloned().unwrap_or_else(|| key.to_string())
    }

    #[must_use]
    pub fn fill(&self, key: &str, values: &[(&str, &str)]) -> String {
        let mut text = self.get(key);
        for (name, value) in values {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    fn lang(&self) -> String {
        self.0
            .get("htmlLang")
            .cloned()
            .unwrap_or_else(|| "en".to_string())
    }

    fn dir(&self) -> &'static str {
        if self.0.get("htmlDir").is_some_and(|d| d == "rtl") {
            "rtl"
        } else {
            "ltr"
        }
    }
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn render_html(kit: &RecoveryKit, t: &KitText) -> String {
    use std::fmt::Write as _;

    let h = |key: &str| esc(&t.get(key));
    let yes_no = |b: bool| h(if b { "yes" } else { "no" });
    let mut out = String::new();

    let _ = write!(
        out,
        "<!DOCTYPE html>\n<html lang=\"{}\" dir=\"{}\"><head><meta charset=\"utf-8\">\n\
         <title>{}</title>\n<style>\n",
        esc(&t.lang()),
        t.dir(),
        h("title")
    );
    out.push_str(
        "  @page { margin: 1.5cm; }\n\
         body { font-family: -apple-system, Segoe UI, Roboto, Arial, sans-serif; color: #1a1a1a; \
         font-size: 13px; line-height: 1.5; max-width: 900px; margin: 0 auto; padding: 24px; }\n\
         h1 { font-size: 20px; margin: 0 0 4px; }\n\
         h2 { font-size: 14px; margin: 24px 0 8px; border-bottom: 1px solid #ccc; padding-bottom: 4px; }\n\
         .cu-warning { border: 2px solid #b00020; background: #fdecea; color: #b00020; \
         padding: 10px 14px; border-radius: 6px; font-weight: 600; margin: 12px 0; }\n\
         table { border-collapse: collapse; width: 100%; margin: 4px 0 12px; }\n\
         td, th { border: 1px solid #ccc; padding: 4px 8px; text-align: start; \
         vertical-align: top; font-size: 12px; }\n\
         .cu-meta td:first-child { font-weight: 600; width: 220px; }\n\
         .cu-words { list-style: none; margin: 8px 0; padding: 0; display: grid; \
         grid-template-columns: repeat(4, 1fr); gap: 6px 16px; }\n\
         .cu-words li { border-bottom: 1px solid #666; padding: 2px 4px 10px; font-size: 12px; }\n\
         .cu-idx { color: #666; margin-inline-end: 6px; }\n\
         footer { margin-top: 24px; font-size: 11px; color: #666; }\n\
         @media print { .cu-noprint { display: none; } }\n\
         </style></head><body>\n",
    );

    let _ = writeln!(out, "<h1>{}</h1>", h("title"));
    let _ = writeln!(
        out,
        "<p>{}</p>",
        esc(&t.fill("generatedBy", &[("version", &kit.app_version)]))
    );
    let _ = writeln!(out, "<div class=\"cu-warning\">{}</div>", h("warning"));

    let _ = writeln!(
        out,
        "<h2>{}</h2>\n<table class=\"cu-meta\">",
        h("vaultCoords")
    );
    let mode_row = match &kit.mode {
        VaultMode::LocalOnly => h("modeLocal"),
        VaultMode::LocalPlusSync { remotes } => {
            esc(&t.fill("modeSync", &[("remotes", &remotes.join(", "))]))
        }
        VaultMode::CloudOnly { remote } => esc(&t.fill("modeCloud", &[("remote", remote)])),
    };
    let _ = writeln!(out, "<tr><td>{}</td><td>{mode_row}</td></tr>", h("mode"));
    if let Some(path) = &kit.local_path {
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{}</td></tr>",
            h("localPath"),
            esc(path)
        );
    }
    let _ = writeln!(
        out,
        "<tr><td>{}</td><td>{} / {} / {}</td></tr>",
        h("formats"),
        esc(&kit.file_format),
        esc(&kit.name_format),
        esc(&kit.kdf_name)
    );
    out.push_str("</table>\n");

    let _ = writeln!(
        out,
        "<h2>{}</h2>\n<table class=\"cu-meta\">",
        h("unlockMethods")
    );
    for (key, on) in [
        ("password", kit.has_password),
        ("recoveryPhrase", kit.has_recovery_phrase),
        ("fido2", kit.has_fido2),
        ("keychain", kit.has_platform_unlock),
    ] {
        let _ = writeln!(out, "<tr><td>{}</td><td>{}</td></tr>", h(key), yes_no(on));
    }
    let _ = writeln!(
        out,
        "<tr><td>{}</td><td>{}</td></tr>",
        h("members"),
        if kit.members.is_empty() {
            h("none")
        } else {
            esc(&kit.members.join(", "))
        }
    );
    let _ = writeln!(
        out,
        "<tr><td>{}</td><td>{}</td></tr>",
        h("inbox"),
        yes_no(kit.inbox_active)
    );
    out.push_str("</table>\n");

    let _ = writeln!(out, "<h2>{}</h2>", h("jobs"));
    if kit.jobs.is_empty() {
        let _ = writeln!(out, "<p>{}</p>", h("noJobs"));
    } else {
        let _ = writeln!(
            out,
            "<table><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr>",
            h("jobName"),
            h("jobLocal"),
            h("jobVaultPath"),
            h("jobRemote")
        );
        for job in &kit.jobs {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc(&job.name),
                esc(&job.local_root),
                if job.vault_path.is_empty() {
                    h("root")
                } else {
                    esc(&job.vault_path)
                },
                job.remote_name.as_deref().map_or_else(|| h("local"), esc)
            );
        }
        out.push_str("</table>\n");
    }

    let _ = writeln!(out, "<h2>{}</h2>", h("remotes"));
    if kit.remotes.is_empty() {
        let _ = writeln!(out, "<p>{}</p>", h("noRemotes"));
    } else {
        for remote in &kit.remotes {
            match remote {
                RemoteInfo::Known(summary) => {
                    let _ = writeln!(
                        out,
                        "<table class=\"cu-meta\"><tr><td>{}</td><td>{} ({})</td></tr>",
                        h("name"),
                        esc(&summary.name),
                        esc(&summary.provider)
                    );
                    for (label, value) in &summary.details {
                        let _ = writeln!(
                            out,
                            "<tr><td>{}</td><td>{}</td></tr>",
                            esc(label),
                            esc(value)
                        );
                    }
                    out.push_str("</table>\n");
                }
                RemoteInfo::Locked { name } => {
                    let _ = writeln!(
                        out,
                        "<table class=\"cu-meta\"><tr><td>{}</td><td>{}</td></tr>\n\
                         <tr><td>{}</td><td>{}</td></tr></table>",
                        h("name"),
                        esc(name),
                        h("details"),
                        h("remoteLocked")
                    );
                }
            }
        }
    }

    let _ = writeln!(out, "<h2>{}</h2>\n<ol>", h("howTo"));
    for step in 1..=7u8 {
        let _ = writeln!(out, "<li>{}</li>", h(&format!("step{step}")));
    }
    out.push_str("</ol>\n");

    let _ = writeln!(
        out,
        "<h2>{}</h2>\n<ol class=\"cu-words\">",
        h("phraseHeading")
    );
    for i in 1..=24u8 {
        let _ = writeln!(out, "<li><span class=\"cu-idx\">{i}.</span></li>");
    }
    out.push_str("</ol>\n");

    let _ = writeln!(out, "<footer>{}</footer>", h("footer"));
    out.push_str("</body></html>\n");
    out
}
