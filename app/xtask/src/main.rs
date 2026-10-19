// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashMap, HashSet, VecDeque};
use std::process::{Command, exit};

use serde_json::Value;

const ALLOWED: &[(&str, &[&str])] = &[
    ("cu-msg", &[]),
    ("cu-crypto", &[]),
    ("cu-format", &["cu-crypto"]),
    ("cu-backend", &[]),
    ("cu-oauth", &[]),
    ("cu-backend-fs", &["cu-backend"]),
    ("cu-backend-s3", &["cu-backend"]),
    ("cu-backend-gdrive", &["cu-backend", "cu-oauth"]),
    ("cu-backend-dropbox", &["cu-backend", "cu-oauth"]),
    ("cu-backend-onedrive", &["cu-backend", "cu-oauth"]),
    ("cu-backend-sftp", &["cu-backend"]),
    ("cu-backend-webdav", &["cu-backend"]),
    ("cu-auth-fido2", &[]),
    ("cu-vfs", &["cu-crypto", "cu-format", "cu-backend"]),
    ("cu-index", &[]),
    (
        "cu-sync",
        &["cu-crypto", "cu-index", "cu-vfs", "cu-backend", "cu-format"],
    ),
    ("cu-jobs", &["cu-sync", "cu-index"]),
    ("cu-secrets", &["cu-crypto"]),
    ("cu-sentinel", &[]),
    (
        "cu-core",
        &[
            "cu-crypto",
            "cu-format",
            "cu-backend",
            "cu-backend-fs",
            "cu-backend-s3",
            "cu-backend-gdrive",
            "cu-backend-dropbox",
            "cu-backend-onedrive",
            "cu-backend-sftp",
            "cu-backend-webdav",
            "cu-oauth",
            "cu-auth-fido2",
            "cu-vfs",
            "cu-index",
            "cu-sync",
            "cu-jobs",
            "cu-secrets",
            "cu-sentinel",
            "cu-mount",
        ],
    ),
    (
        "cu-mount",
        &["cu-vfs", "cu-format", "cu-crypto", "cu-backend"],
    ),
    ("cryptoupload-desktop", &["cu-core"]),
    ("xtask", &[]),
];

const HTTP_EXEMPT: &[&str] = &[
    "cu-backend-s3",
    "cu-backend-gdrive",
    "cu-backend-dropbox",
    "cu-backend-onedrive",
    "cu-backend-sftp",
    "cu-backend-webdav",
    "cu-oauth",
    "cryptoupload-desktop",
];

const HTTP_CRATES: &[&str] = &[
    "reqwest",
    "reqwest-middleware",
    "reqwest-retry",
    "hyper",
    "hyper-util",
    "hyper-rustls",
    "hyper-tls",
    "ureq",
    "curl",
    "curl-sys",
    "isahc",
    "tonic",
    "h2",
    "surf",
    "attohttpc",
];

const WINDOWS_TARGET: &str = "x86_64-pc-windows-gnu";

const WINDOWS_CHECK_EXCLUDE: &[&str] = &["cu-mount", "cu-core", "cryptoupload-desktop"];

fn main() {
    let cmd = std::env::args().nth(1);
    let run_layering = matches!(cmd.as_deref(), None | Some("check-layering"));
    let run_no_http = matches!(cmd.as_deref(), None | Some("check-no-http"));
    let run_windows = matches!(cmd.as_deref(), Some("check-windows"));
    let run_codes = matches!(cmd.as_deref(), Some("check-error-codes"));

    if !run_layering && !run_no_http && !run_windows && !run_codes {
        eprintln!(
            "usage: xtask [check-layering | check-no-http | check-error-codes | check-windows]"
        );
        exit(2);
    }

    let mut ok = true;

    if run_layering || run_no_http {
        let meta = cargo_metadata();
        if run_layering {
            ok &= check_layering(&meta);
        }
        if run_no_http {
            ok &= check_no_http(&meta);
        }
    }

    if run_windows {
        ok &= check_windows();
    }

    if run_codes {
        ok &= check_error_codes();
    }

    if ok {
        println!("OK");
    } else {
        exit(1);
    }
}

const FIXED_CODES: &[&str] = &[
    "CONFIG_ROLLED_BACK",
    "REMOTE_SESSION",
    "NO_VAULT_UNLOCKED",
    "TRANSLATION_FILE_TOO_LARGE",
    "TRANSLATION_FILE_INVALID",
    "S3_HTTP_WARNING",
];
const CODE_PREFIXES: &[&str] = &["SEC_", "PRESET_NOTE_", "JOBS_THRESHOLD_"];
const CODE_SKIP_DIRS: &[&str] = &["target", "node_modules"];

fn is_code(text: &str) -> bool {
    text.len() >= 3
        && text.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && text
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn collect_rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !CODE_SKIP_DIRS.contains(&name.as_str()) {
                collect_rust_files(&path, out);
            }
        } else if std::path::Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
        {
            out.push(path);
        }
    }
}

fn quoted_after(text: &str, marker: &str, out: &mut std::collections::BTreeSet<String>) {
    let mut rest = text;
    while let Some(i) = rest.find(marker) {
        rest = &rest[i + marker.len()..];
        let trimmed = rest.trim_start();
        if let Some(body) = trimmed.strip_prefix('"')
            && let Some(end) = body.find('"')
            && is_code(&body[..end])
        {
            out.insert(body[..end].to_string());
        }
    }
}

fn check_error_codes() -> bool {
    use std::collections::BTreeSet;
    let app = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let dictionary = std::fs::read_to_string(app.join("apps/desktop/src/lib/i18n/locales/en.ts"))
        .expect("English dictionary not readable");
    let mut in_codes = false;
    let mut dictionary_codes = BTreeSet::new();
    for line in dictionary.lines() {
        if line.starts_with("\t\tcodes: {") {
            in_codes = true;
        } else if in_codes && line.starts_with("\t\t},") {
            break;
        } else if in_codes
            && let Some(code) = line
                .strip_prefix("\t\t\t")
                .and_then(|l| l.split(':').next())
            && is_code(code)
        {
            dictionary_codes.insert(code.to_string());
        }
    }

    let mut files = Vec::new();
    collect_rust_files(&app.join("crates"), &mut files);
    collect_rust_files(&app.join("apps/desktop/src-tauri/src"), &mut files);
    let mut used = BTreeSet::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let text = text.as_str();
        quoted_after(text, "Msg::new(", &mut used);
        for prefix in CODE_PREFIXES {
            let mut rest = text;
            let needle = format!("\"{prefix}");
            while let Some(i) = rest.find(&needle) {
                rest = &rest[i + 1..];
                if let Some(end) = rest.find('"')
                    && is_code(&rest[..end])
                {
                    used.insert(rest[..end].to_string());
                }
            }
        }
    }
    used.extend(FIXED_CODES.iter().map(|c| (*c).to_string()));

    let mut ok = true;
    for code in used.difference(&dictionary_codes) {
        eprintln!(
            "xtask: code `{code}` is used by the backend but missing from errors.codes (en.ts)"
        );
        ok = false;
    }
    let legacy: &[&str] = &[
        "PATH_NOT_FOUND",
        "DESTINATION_EXISTS",
        "NOT_A_VAULT",
        "DECRYPT_FAILED",
        "OBJECT_NOT_FOUND",
        "OBJECT_EXISTS",
        "TRANSIENT",
        "IO",
        "WEAK_PASSWORD",
        "TWO_FACTOR_REQUIRED",
        "POLICY_REAUTH_REQUIRED",
        "CONFIG_CHANGED_ELSEWHERE",
        "NO_RECOVERY_PHRASE",
        "VERSION_NOT_FOUND",
        "CONTENT_UNAVAILABLE",
        "RANDOM_UNAVAILABLE",
        "REGISTRY_DISABLED",
        "PLATFORM_KEY_UNAVAILABLE",
    ];
    for code in dictionary_codes.difference(&used) {
        if !legacy.contains(&code.as_str()) {
            eprintln!(
                "xtask: code `{code}` is in the dictionary but the backend no longer uses it"
            );
            ok = false;
        }
    }
    println!(
        "check-error-codes: {} codes used, {} dictionary entries",
        used.len(),
        dictionary_codes.len()
    );
    ok
}

fn check_windows() -> bool {
    let mut args = vec!["check", "--workspace", "--target", WINDOWS_TARGET];
    for exclude in WINDOWS_CHECK_EXCLUDE {
        args.push("--exclude");
        args.push(exclude);
    }
    let status = Command::new("cargo").args(&args).status();

    match status {
        Ok(status) => status.success(),
        Err(e) => {
            eprintln!(
                "xtask: cannot run `cargo check --target {WINDOWS_TARGET}`: {e}\n\
                 It needs the `{WINDOWS_TARGET}` target (`rustup target add {WINDOWS_TARGET}`) and a \
                 MinGW-w64 cross linker (`x86_64-w64-mingw32-gcc`) on the PATH."
            );
            false
        }
    }
}

fn cargo_metadata() -> Value {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version=1",
            "--filter-platform",
            &host_triple(),
        ])
        .output()
        .expect("cannot run `cargo metadata`: is it in PATH?");

    if !output.status.success() {
        eprintln!("cargo metadata failed:");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        exit(1);
    }

    serde_json::from_slice(&output.stdout).expect("`cargo metadata` output is not valid JSON")
}

fn host_triple() -> String {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("cannot run `rustc -vV`: is it in PATH?");
    let stdout = String::from_utf8(output.stdout).expect("`rustc -vV` output is not UTF-8");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("`host:` line missing from `rustc -vV` output")
        .trim()
        .to_string()
}

fn workspace_member_ids(meta: &Value) -> HashSet<String> {
    meta["workspace_members"]
        .as_array()
        .expect("workspace_members missing from cargo metadata output")
        .iter()
        .map(|id| id.as_str().expect("id is not a string").to_owned())
        .collect()
}

fn id_to_name(meta: &Value) -> HashMap<String, String> {
    meta["packages"]
        .as_array()
        .expect("packages missing from cargo metadata output")
        .iter()
        .map(|pkg| {
            let id = pkg["id"].as_str().expect("id is not a string").to_owned();
            let name = pkg["name"]
                .as_str()
                .expect("name is not a string")
                .to_owned();
            (id, name)
        })
        .collect()
}

fn check_layering(meta: &Value) -> bool {
    let member_ids = workspace_member_ids(meta);
    let internal_names: HashSet<&str> = ALLOWED.iter().map(|(name, _)| *name).collect();

    let mut ok = true;

    for pkg in meta["packages"].as_array().expect("packages missing") {
        let id = pkg["id"].as_str().expect("id is not a string");
        if !member_ids.contains(id) {
            continue;
        }
        let name = pkg["name"].as_str().expect("name is not a string");

        let Some((_, allowed_deps)) = ALLOWED.iter().find(|(n, _)| *n == name) else {
            eprintln!(
                "xtask: crate `{name}` is not listed in the ALLOWED table of app/xtask/src/main.rs \
                 (add it to the table of allowed dependencies too)"
            );
            ok = false;
            continue;
        };

        for dep in pkg["dependencies"].as_array().unwrap_or(&Vec::new()) {
            if dep["kind"] != Value::Null {
                continue;
            }
            let dep_name = dep["name"].as_str().expect("dep name is not a string");
            if !internal_names.contains(dep_name) {
                continue;
            }
            if dep_name == "cu-msg" {
                continue;
            }
            if !allowed_deps.contains(&dep_name) {
                eprintln!(
                    "xtask: layering violation: `{name}` depends on `{dep_name}`, not \
                     allowed by the table of allowed dependencies"
                );
                ok = false;
            }
        }
    }

    ok
}

fn check_no_http(meta: &Value) -> bool {
    let member_ids = workspace_member_ids(meta);
    let names = id_to_name(meta);

    let Some(nodes) = meta["resolve"]["nodes"].as_array() else {
        eprintln!("xtask: `resolve.nodes` missing from cargo metadata output");
        return false;
    };

    let adjacency: HashMap<String, Vec<String>> = nodes
        .iter()
        .map(|node| {
            let id = node["id"].as_str().expect("id is not a string").to_owned();
            let deps = node["dependencies"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|d| d.as_str().expect("dep id is not a string").to_owned())
                        .collect()
                })
                .unwrap_or_default();
            (id, deps)
        })
        .collect();

    let mut ok = true;

    for member_id in &member_ids {
        let member_name = names.get(member_id).map_or("<unknown>", String::as_str);
        if HTTP_EXEMPT.contains(&member_name) {
            continue;
        }

        let mut seen: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        queue.push_back(member_id.as_str());
        seen.insert(member_id.as_str());

        while let Some(id) = queue.pop_front() {
            let id_name = names.get(id).map_or("<unknown>", String::as_str);
            if id != member_id.as_str() && HTTP_EXEMPT.contains(&id_name) {
                continue;
            }
            let Some(deps) = adjacency.get(id) else {
                continue;
            };
            for dep_id in deps {
                if seen.insert(dep_id.as_str()) {
                    queue.push_back(dep_id.as_str());
                }
            }
        }

        for id in &seen {
            if *id == member_id {
                continue;
            }
            let dep_name = names.get(*id).map_or("<unknown>", String::as_str);
            if HTTP_CRATES.contains(&dep_name) {
                eprintln!(
                    "xtask: `{member_name}` pulls in a forbidden HTTP dependency: `{dep_name}` \
                     (direct or transitive, not through a network backend). Only \
                     the cu-backend-* network crates may have their own."
                );
                ok = false;
            }
        }
    }

    ok
}
