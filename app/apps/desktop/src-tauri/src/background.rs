// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, Window, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;

use crate::guard::{self, LockReason};
use crate::settings::TrayTexts;
use crate::state::AppState;

pub const BACKGROUND_ARG: &str = "--background";

const RELOAD_EVERY: Duration = Duration::from_secs(15);

const ID_OPEN: &str = "open";
const ID_QUIT: &str = "quit";

static TRAY_READY: AtomicBool = AtomicBool::new(false);

struct Tray {
    _icon: TrayIcon,
    open: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

static TRAY: Mutex<Option<Tray>> = Mutex::new(None);

pub fn set_start_at_login(app: &AppHandle, enable: bool) -> bool {
    let manager = app.autolaunch();
    let result = if enable {
        manager.enable()
    } else {
        manager.disable()
    };
    result.is_ok()
}

pub fn show_tray(app: &AppHandle, texts: &TrayTexts) {
    let handle = app.clone();
    let texts = texts.clone();
    let _ = app.run_on_main_thread(move || {
        let mut slot = TRAY.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(tray) = slot.as_ref() {
            let _ = tray.open.set_text(&texts.open);
            let _ = tray.quit.set_text(&texts.quit);
            return;
        }
        match build_tray(&handle, &texts) {
            Ok(tray) => {
                *slot = Some(tray);
                TRAY_READY.store(true, Ordering::SeqCst);
            }
            Err(e) => eprintln!("tray icon not created: {e}"),
        }
    });
}

fn build_tray(app: &AppHandle, texts: &TrayTexts) -> tauri::Result<Tray> {
    let open = MenuItem::with_id(app, ID_OPEN, &texts.open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, &texts.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            ID_OPEN => open_main_window(app),
            ID_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    Ok(Tray {
        _icon: builder.build(app)?,
        open,
        quit,
    })
}

pub fn open_main_window(app: &AppHandle) {
    if let Some(window) = app.webview_windows().into_values().next() {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let hidden = app
            .state::<AppState>()
            .settings
            .lock()
            .await
            .security
            .hide_from_capture;
        if let Err(e) = crate::webview_recycle::open_fresh(&app, hidden) {
            eprintln!("window not reopened: {e}");
        }
    });
}

async fn keeps_running(state: &AppState) -> bool {
    if !TRAY_READY.load(Ordering::SeqCst) || !state.settings.lock().await.run_in_background {
        return false;
    }
    #[cfg(any(target_os = "linux", windows))]
    if state.mount.lock().await.is_some() {
        return false;
    }
    true
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if !window.label().starts_with("main") {
        return;
    }
    api.prevent_close();
    let app = window.app_handle().clone();
    let window = window.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        if keeps_running(&state).await {
            guard::force_lock(&state, LockReason::WindowClosed, guard::FLUSH_BUDGET).await;
            let capture_hidden = state.settings.lock().await.security.hide_from_capture;
            match crate::webview_recycle::replace_hidden(&app, capture_hidden).await {
                Ok(()) => tell_once(&app, &state).await,
                Err(e) => eprintln!("window not replaced: {e}"),
            }
        } else {
            let _ = window.destroy();
        }
    });
}

async fn tell_once(app: &AppHandle, state: &AppState) {
    use tauri_plugin_notification::NotificationExt as _;

    let mut settings = state.settings.lock().await;
    if settings.background_hint_seen {
        return;
    }
    let Some(texts) = settings.tray_texts.clone() else {
        return;
    };
    settings.background_hint_seen = true;
    let _ = crate::settings::save(&state.data_dir, &settings);
    drop(settings);
    let _ = app
        .notification()
        .builder()
        .title("CryptoUpload")
        .body(texts.notice)
        .show();
}

pub fn on_run_event(app: &AppHandle, event: &RunEvent) {
    let RunEvent::ExitRequested {
        code: None, api, ..
    } = event
    else {
        return;
    };
    let state = app.state::<AppState>();
    let keep =
        TRAY_READY.load(Ordering::SeqCst) && state.settings.blocking_lock().run_in_background && {
            #[cfg(any(target_os = "linux", windows))]
            {
                state.mount.blocking_lock().is_none()
            }
            #[cfg(not(any(target_os = "linux", windows)))]
            {
                true
            }
        };
    if keep {
        api.prevent_exit();
    }
}

pub fn startup(app: &AppHandle, background_start: bool) {
    let state = app.state::<AppState>();
    let (texts, run_in_background) = {
        let settings = state.settings.blocking_lock();
        (settings.tray_texts.clone(), settings.run_in_background)
    };
    if let Some(texts) = &texts {
        let mut slot = TRAY.lock().unwrap_or_else(PoisonError::into_inner);
        if let Ok(tray) = build_tray(app, texts) {
            *slot = Some(tray);
            TRAY_READY.store(true, Ordering::SeqCst);
        }
    }
    if !(background_start && run_in_background && TRAY_READY.load(Ordering::SeqCst))
        && let Some(window) = app.get_webview_window("main")
    {
        let _ = window.show();
    }
}

pub fn start_scheduler(app: &AppHandle) {
    let data_dir = app.state::<AppState>().data_dir.clone();
    tauri::async_runtime::spawn(async move {
        let store = cu_core::JobStore::new(cu_core::default_jobs_path(&data_dir));
        let mut running: Option<Running> = None;
        loop {
            if let Ok(all) = store.load() {
                let enabled: Vec<_> = all.into_iter().filter(|j| j.enabled).collect();
                let unchanged = running.as_ref().is_some_and(|r| r.jobs == enabled)
                    || (running.is_none() && enabled.is_empty());
                if !unchanged {
                    if let Some(old) = running.take() {
                        let _ = old.stop.send(true);
                        let _ = old.task.await;
                    }
                    if !enabled.is_empty() {
                        running = Some(Running::start(&data_dir, enabled));
                    }
                }
            }
            tokio::time::sleep(RELOAD_EVERY).await;
        }
    });
}

struct Running {
    jobs: Vec<cu_core::JobSpec>,
    stop: tokio::sync::watch::Sender<bool>,
    task: tauri::async_runtime::JoinHandle<()>,
}

impl Running {
    fn start(data_dir: &std::path::Path, jobs: Vec<cu_core::JobSpec>) -> Self {
        let (stop, shutdown) = tokio::sync::watch::channel(false);
        let manager = cu_core::JobManager::new(
            cu_core::SyncJobRunner::new(data_dir.to_path_buf()),
            cu_core::StatusStore::new(cu_core::default_status_dir(data_dir)),
        );
        let to_run = jobs.clone();
        let task = tauri::async_runtime::spawn(async move {
            manager.run_until(to_run, shutdown).await;
        });
        Self { jobs, stop, task }
    }
}
