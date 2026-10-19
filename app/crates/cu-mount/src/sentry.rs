// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MASS_READ_WINDOW: Duration = Duration::from_secs(10);

pub const ASK_TIMEOUT: Duration = Duration::from_secs(30);

pub const DECOY_COUNT: usize = 2;

const DECOY_FAMILIES: &[(&[&str], &[&str])] = &[
    (
        &[
            "passwords",
            "accounts-backup",
            "logins",
            "vault-export",
            "credentials",
            "keys-backup",
            "password-list",
        ],
        &["kdbx", "csv", "json", "txt", "xlsx"],
    ),
    (
        &[
            "wallet-seed",
            "seed-phrase",
            "wallet-backup",
            "recovery-words",
            "crypto-keys",
            "cold-wallet",
        ],
        &["txt", "dat", "json", "pdf"],
    ),
    (
        &[
            "bank-statements",
            "tax-return",
            "passport-scan",
            "id-card",
            "payroll",
            "invoices",
            "contracts",
            "medical-records",
        ],
        &["pdf", "xlsx", "zip", "docx"],
    ),
    (
        &[
            "private-notes",
            "diary",
            "photos-backup",
            "phone-backup",
            "documents-archive",
        ],
        &["zip", "docx", "txt", "7z"],
    ),
];

const DECOY_SUFFIXES: &[&str] = &[
    "", "", "-old", "-copy", "-final", "-2023", "-2024", "-2025", "-2026", "_v2", "-new",
];

#[must_use]
pub fn pick_decoy_names() -> Vec<String> {
    let mut random = [0u8; 4 * DECOY_COUNT];
    if cu_crypto::random::fill_random(&mut random).is_err() {
        use std::hash::{BuildHasher as _, Hasher as _};
        for (i, byte) in random.iter_mut().enumerate() {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_usize(i);
            *byte = hasher.finish().to_le_bytes().first().copied().unwrap_or(0);
        }
    }
    let mut families: Vec<usize> = (0..DECOY_FAMILIES.len()).collect();
    let mut names = Vec::with_capacity(DECOY_COUNT);
    for pick in random.as_chunks::<4>().0.iter().take(DECOY_COUNT) {
        let [a, b, c, d] = pick.map(usize::from);
        if families.is_empty() {
            break;
        }
        let family = families.remove(a % families.len());
        let Some((stems, extensions)) = DECOY_FAMILIES.get(family) else {
            continue;
        };
        let stem = stems.get(b % stems.len()).copied().unwrap_or("backup");
        let suffix = DECOY_SUFFIXES
            .get(c % DECOY_SUFFIXES.len())
            .copied()
            .unwrap_or("");
        let extension = extensions
            .get(d % extensions.len())
            .copied()
            .unwrap_or("txt");
        names.push(format!(".{stem}{suffix}.{extension}"));
    }
    names
}

pub const DECOY_SIZE: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Access {
    #[default]
    Off,
    Allowlist,
    Ask,
}

#[derive(Debug, Clone, Default)]
pub struct SentryRules {
    pub access: Access,
    pub allowed: Vec<String>,
    pub decoys: bool,
    pub mass_read_threshold: u32,
    pub exempt: Vec<String>,
}

impl SentryRules {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.access != Access::Off || self.decoys || self.mass_read_threshold != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tripwire {
    Decoy {
        program: Option<String>,
    },
    MassRead {
        program: Option<String>,
        files: usize,
    },
}

pub type AskFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

pub trait SentryEvents: Send + Sync {
    fn denied(&self, program: &str);
    fn ask<'a>(&'a self, program: &'a str) -> AskFuture<'a>;
    fn tripwire(&self, trip: Tripwire);
}

#[derive(Default)]
struct State {
    answered: HashMap<String, bool>,
    opens: HashMap<u32, VecDeque<(Instant, u64)>>,
    tripped: bool,
}

pub struct Sentry {
    rules: SentryRules,
    decoy_names: Vec<String>,
    events: Option<Arc<dyn SentryEvents>>,
    own_pid: u32,
    state: Mutex<State>,
}

impl std::fmt::Debug for Sentry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sentry")
            .field("rules", &self.rules)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "linux")]
#[must_use]
pub fn program_of(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(not(target_os = "linux"))]
#[must_use]
pub fn program_of(_pid: u32) -> Option<String> {
    None
}

impl Sentry {
    #[must_use]
    pub fn new(rules: SentryRules, events: Option<Arc<dyn SentryEvents>>) -> Self {
        let decoy_names = if rules.decoys {
            pick_decoy_names()
        } else {
            Vec::new()
        };
        Self {
            rules,
            decoy_names,
            events,
            own_pid: std::process::id(),
            state: Mutex::new(State::default()),
        }
    }

    #[must_use]
    pub fn rules(&self) -> &SentryRules {
        &self.rules
    }

    #[must_use]
    pub fn is_decoy(&self, name: &str) -> bool {
        self.decoy_position(name).is_some()
    }

    #[must_use]
    pub fn decoy_names(&self) -> &[String] {
        &self.decoy_names
    }

    #[must_use]
    pub fn decoy_position(&self, name: &str) -> Option<usize> {
        if !self.rules.decoys {
            return None;
        }
        self.decoy_names.iter().position(|decoy| decoy == name)
    }

    fn is_internal(&self, pid: u32) -> bool {
        pid == 0 || pid == self.own_pid
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub async fn admits(&self, pid: u32) -> bool {
        if self.rules.access == Access::Off || self.is_internal(pid) {
            return true;
        }
        let Some(program) = program_of(pid) else {
            return false;
        };
        self.admits_program(&program).await
    }

    pub async fn admits_program(&self, program: &str) -> bool {
        if self.rules.allowed.iter().any(|allowed| allowed == program) {
            return true;
        }
        if self.rules.access == Access::Ask {
            if let Some(answer) = self.lock().answered.get(program).copied() {
                return answer;
            }
            let answer = match &self.events {
                Some(events) => tokio::time::timeout(ASK_TIMEOUT, events.ask(program))
                    .await
                    .unwrap_or(false),
                None => false,
            };
            self.lock().answered.insert(program.to_string(), answer);
            if answer {
                return true;
            }
        }
        if let Some(events) = &self.events {
            events.denied(program);
        }
        false
    }

    fn is_exempt(&self, pid: u32, program: Option<&str>) -> bool {
        self.is_internal(pid)
            || program.is_some_and(|p| self.rules.exempt.iter().any(|exempt| exempt == p))
    }

    fn trip(&self, trip: Tripwire) {
        {
            let mut state = self.lock();
            if state.tripped {
                return;
            }
            state.tripped = true;
        }
        if let Some(events) = &self.events {
            events.tripwire(trip);
        }
    }

    pub fn decoy_touched(&self, pid: u32) {
        let program = program_of(pid);
        if !self.rules.decoys || self.is_exempt(pid, program.as_deref()) {
            return;
        }
        self.trip(Tripwire::Decoy { program });
    }

    pub fn file_opened(&self, pid: u32, ino: u64, now: Instant) {
        let threshold = usize::try_from(self.rules.mass_read_threshold).unwrap_or(usize::MAX);
        if threshold == 0 || self.is_internal(pid) {
            return;
        }
        let distinct = {
            let mut state = self.lock();
            state.opens.retain(|_, opens| {
                opens
                    .back()
                    .is_some_and(|(at, _)| now.duration_since(*at) < MASS_READ_WINDOW)
            });
            let opens = state.opens.entry(pid).or_default();
            while opens
                .front()
                .is_some_and(|(at, _)| now.duration_since(*at) >= MASS_READ_WINDOW)
            {
                opens.pop_front();
            }
            opens.push_back((now, ino));
            opens
                .iter()
                .map(|(_, ino)| *ino)
                .collect::<HashSet<_>>()
                .len()
        };
        if distinct <= threshold {
            return;
        }
        let program = program_of(pid);
        if self.is_exempt(pid, program.as_deref()) {
            return;
        }
        self.trip(Tripwire::MassRead {
            program,
            files: distinct,
        });
    }
}
