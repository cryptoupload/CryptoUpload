// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, WebviewWindowBuilder};

static GENERATION: AtomicU32 = AtomicU32::new(0);

const SETTLE: Duration = Duration::from_millis(150);

pub fn schedule(app: &AppHandle, hidden_from_capture: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SETTLE).await;
        if let Err(e) = recycle(&app, hidden_from_capture) {
            eprintln!("window not recreated after the lock: {e}");
        }
    });
}

fn recycle(app: &AppHandle, hidden_from_capture: bool) -> tauri::Result<()> {
    let old: Vec<_> = app.webview_windows().into_values().collect();
    let geometry = old.first().and_then(|window| {
        Some((
            window.outer_position().ok()?,
            window.inner_size().ok()?,
            window.is_maximized().unwrap_or(false),
        ))
    });
    build(app, hidden_from_capture, geometry, true)?;
    for window in old {
        window.destroy()?;
    }
    Ok(())
}

pub fn open_fresh(app: &AppHandle, hidden_from_capture: bool) -> tauri::Result<()> {
    build(app, hidden_from_capture, None, true)
}

pub async fn replace_hidden(app: &AppHandle, hidden_from_capture: bool) -> tauri::Result<()> {
    let old: Vec<_> = app.webview_windows().into_values().collect();
    let stale = web_processes();
    build(app, hidden_from_capture, None, false)?;
    for window in old {
        window.destroy()?;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    for pid in stale {
        end_process(pid);
    }
    Ok(())
}

fn web_processes() -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        let me = std::process::id();
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| {
                let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                    return false;
                };
                let Some((head, tail)) = stat.rsplit_once(')') else {
                    return false;
                };
                let ppid = tail
                    .split_whitespace()
                    .nth(1)
                    .and_then(|p| p.parse::<u32>().ok());
                ppid == Some(me) && head.contains("(WebKitWebProces")
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

fn end_process(pid: u32) {
    #[cfg(unix)]
    let _ = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
    #[cfg(not(unix))]
    let _ = pid;
}

type Geometry = (tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>, bool);

fn build(
    app: &AppHandle,
    hidden_from_capture: bool,
    geometry: Option<Geometry>,
    show: bool,
) -> tauri::Result<()> {
    let Some(mut conf) = app.config().app.windows.first().cloned() else {
        return Ok(());
    };
    conf.label = format!("main-{}", GENERATION.fetch_add(1, Ordering::SeqCst) + 1);
    let fresh = WebviewWindowBuilder::from_config(app, &conf)?
        .visible(false)
        .build()?;
    if let Some((position, size, maximized)) = geometry {
        let _ = fresh.set_position(position);
        let _ = fresh.set_size(size);
        if maximized {
            let _ = fresh.maximize();
        }
    }
    let _ = fresh.set_content_protected(hidden_from_capture);
    if show {
        fresh.show()?;
        let _ = fresh.set_focus();
    }
    Ok(())
}
