// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedFd, OwnedObjectPath, OwnedValue};

use crate::{SentinelError, SessionEvent};

const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const USER: &str = "org.freedesktop.login1.User";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

pub type Listener = Arc<dyn Fn(SessionEvent) + Send + Sync>;

fn unavailable(e: impl std::fmt::Display) -> SentinelError {
    SentinelError::Unavailable(e.to_string())
}

fn manager(connection: &Connection) -> Result<Proxy<'static>, SentinelError> {
    Proxy::new(connection, LOGIND, MANAGER_PATH, MANAGER).map_err(unavailable)
}

fn current_uid() -> Result<u32, SentinelError> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|m| m.uid())
        .map_err(unavailable)
}

pub fn session_path(connection: &Connection) -> Result<OwnedObjectPath, SentinelError> {
    let manager = manager(connection)?;
    let display = current_uid().and_then(|uid| {
        let user_path: OwnedObjectPath = manager.call("GetUser", &(uid,)).map_err(unavailable)?;
        let user = Proxy::new(connection, LOGIND, user_path, USER).map_err(unavailable)?;
        let (_id, path): (String, OwnedObjectPath) =
            user.get_property("Display").map_err(unavailable)?;
        Ok(path)
    });
    match display {
        Ok(path) if path.as_str() != "/" => Ok(path),
        _ => manager
            .call("GetSessionByPID", &(std::process::id(),))
            .map_err(unavailable),
    }
}

pub fn session_is_remote(connection: &Connection) -> Result<bool, SentinelError> {
    let path = session_path(connection)?;
    let session = Proxy::new(connection, LOGIND, path, SESSION).map_err(unavailable)?;
    session.get_property("Remote").map_err(unavailable)
}

fn take_sleep_delay(connection: &Connection) -> Option<OwnedFd> {
    let manager = manager(connection).ok()?;
    let why = crate::sleep_reason();
    manager
        .call("Inhibit", &("sleep", "CryptoUpload", why.as_str(), "delay"))
        .ok()
}

pub fn watch(connection: &Connection, listener: Listener) -> Result<(), SentinelError> {
    let session = session_path(connection)?;

    let manager_proxy = manager(connection)?;
    let sleep_signals = manager_proxy
        .receive_signal("PrepareForSleep")
        .map_err(unavailable)?;
    let delay = Arc::new(Mutex::new(take_sleep_delay(connection)));
    {
        let listener = Arc::clone(&listener);
        let connection = connection.clone();
        std::thread::Builder::new()
            .name("cu-sentinel-sleep".into())
            .spawn(move || {
                for message in sleep_signals {
                    let Ok(going_to_sleep) = message.body().deserialize::<bool>() else {
                        continue;
                    };
                    if going_to_sleep {
                        listener(SessionEvent::Suspending);
                        if let Ok(mut held) = delay.lock() {
                            held.take();
                        }
                    } else {
                        if let Ok(mut held) = delay.lock() {
                            *held = take_sleep_delay(&connection);
                        }
                        listener(SessionEvent::Resumed);
                    }
                }
            })
            .map_err(unavailable)?;
    }

    let session_proxy =
        Proxy::new(connection, LOGIND, session.clone(), SESSION).map_err(unavailable)?;
    let lock_signals = session_proxy.receive_signal("Lock").map_err(unavailable)?;
    {
        let listener = Arc::clone(&listener);
        std::thread::Builder::new()
            .name("cu-sentinel-lock".into())
            .spawn(move || {
                for _ in lock_signals {
                    listener(SessionEvent::ScreenLocked);
                }
            })
            .map_err(unavailable)?;
    }

    let properties_proxy =
        Proxy::new(connection, LOGIND, session, PROPERTIES).map_err(unavailable)?;
    let property_signals = properties_proxy
        .receive_signal("PropertiesChanged")
        .map_err(unavailable)?;
    std::thread::Builder::new()
        .name("cu-sentinel-props".into())
        .spawn(move || {
            for message in property_signals {
                let body = message.body();
                let Ok((interface, changed, _invalidated)) =
                    body.deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                else {
                    continue;
                };
                if interface != SESSION {
                    continue;
                }
                for event in events_of(&changed) {
                    listener(event);
                }
            }
        })
        .map_err(unavailable)?;

    Ok(())
}

fn bool_property(changed: &HashMap<String, OwnedValue>, name: &str) -> Option<bool> {
    changed.get(name).and_then(|v| bool::try_from(v).ok())
}

fn events_of(changed: &HashMap<String, OwnedValue>) -> Vec<SessionEvent> {
    let mut events = Vec::new();
    if bool_property(changed, "LockedHint") == Some(true) {
        events.push(SessionEvent::ScreenLocked);
    }
    if bool_property(changed, "Active") == Some(false) {
        events.push(SessionEvent::SessionInactive);
    }
    events
}
