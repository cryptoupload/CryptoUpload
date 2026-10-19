// SPDX-License-Identifier: GPL-3.0-or-later

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RemoteEvidence(pub String);

#[cfg(target_os = "linux")]
const KNOWN_PROGRAMS: &[(&str, &str)] = &[
    ("anydesk", "AnyDesk"),
    ("teamviewer", "TeamViewer"),
    ("teamviewer_desk", "TeamViewer"),
    ("x11vnc", "x11vnc"),
    ("xvnc", "VNC"),
    ("xtigervnc", "TigerVNC"),
    ("x0vncserver", "TigerVNC"),
    ("xrdp-chansrv", "xrdp"),
    ("krfb", "Krfb"),
    ("rustdesk", "RustDesk"),
    ("nxnode.bin", "NoMachine"),
    ("chrome-remote-d", "Chrome Remote Desktop"),
    ("sunshine", "Sunshine"),
];

#[cfg(windows)]
const KNOWN_PROGRAMS: &[(&str, &str)] = &[
    ("anydesk", "AnyDesk"),
    ("teamviewer", "TeamViewer"),
    ("teamviewer_desktop", "TeamViewer"),
    ("tvnserver", "TightVNC"),
    ("winvnc", "VNC"),
    ("winvnc4", "VNC"),
    ("vncserver", "VNC"),
    ("rustdesk", "RustDesk"),
    ("remoting_host", "Chrome Remote Desktop"),
    ("nxnode.bin", "NoMachine"),
    ("sunshine", "Sunshine"),
    ("msra", "Windows Remote Assistance"),
    ("quickassist", "Quick Assist"),
];

#[cfg(not(any(target_os = "linux", windows)))]
const KNOWN_PROGRAMS: &[(&str, &str)] = &[
    ("anydesk", "AnyDesk"),
    ("teamviewer", "TeamViewer"),
    ("rustdesk", "RustDesk"),
    ("screensharingd", "macOS Screen Sharing"),
    ("sunshine", "Sunshine"),
];

fn known_among<'a>(process_names: impl Iterator<Item = &'a str>) -> Vec<RemoteEvidence> {
    let mut found: Vec<RemoteEvidence> = process_names
        .filter_map(|name| {
            let name = name.trim().to_ascii_lowercase();
            let name = name.strip_suffix(".exe").unwrap_or(&name);
            KNOWN_PROGRAMS
                .iter()
                .find(|(process, _)| *process == name)
                .map(|(_, shown)| RemoteEvidence((*shown).to_string()))
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

#[must_use]
pub fn remote_control_evidence() -> Vec<RemoteEvidence> {
    let mut evidence = platform_evidence();
    evidence.sort();
    evidence.dedup();
    evidence
}

#[cfg(target_os = "linux")]
fn platform_evidence() -> Vec<RemoteEvidence> {
    let names: Vec<String> = std::fs::read_dir("/proc")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
                })
                .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
                .collect()
        })
        .unwrap_or_default();
    let mut evidence = known_among(names.iter().map(String::as_str));
    let remote_session = zbus::blocking::Connection::system()
        .ok()
        .and_then(|connection| crate::logind::session_is_remote(&connection).ok())
        .unwrap_or(false);
    if remote_session {
        evidence.push(RemoteEvidence("logind remote session".to_string()));
    }
    evidence
}

#[cfg(windows)]
fn tasklist_row(line: &str) -> Option<(&str, &str)> {
    let mut fields = line.split("\",\"");
    let name = fields.next()?.trim_start_matches('"');
    let _pid = fields.next()?;
    let session = fields.next()?;
    Some((name, session))
}

#[cfg(windows)]
fn platform_evidence() -> Vec<RemoteEvidence> {
    let Ok(output) = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let rows: Vec<(&str, &str)> = text.lines().filter_map(tasklist_row).collect();
    let mut evidence = known_among(rows.iter().map(|(name, _)| *name));
    if rows
        .iter()
        .any(|(_, session)| session.to_ascii_uppercase().starts_with("RDP-"))
    {
        evidence.push(RemoteEvidence("Windows Remote Desktop (RDP)".to_string()));
    }
    evidence
}

#[cfg(not(any(target_os = "linux", windows)))]
fn platform_evidence() -> Vec<RemoteEvidence> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axco", "comm"])
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    known_among(text.lines())
}
