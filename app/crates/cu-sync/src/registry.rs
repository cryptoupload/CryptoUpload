// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cu_crypto::aead::NONCE_LEN;
use cu_format::config::VaultConfig;
use cu_format::manifest::Writer;
use cu_format::manifest_v2::{Acceptance, EntryKey, Head, Node, Op, State};
use cu_index::Db;
use cu_vfs::{NamespaceObserver, VaultFs, VfsError};
use tokio::sync::oneshot;

use cu_msg::{Coded, Msg};

use crate::error::SyncError;
use crate::manifest::{HeadStores, ManifestKeys};
use crate::manifest_log::{LogCtx, LogRead, append_ops, check_head, read_log};

#[derive(Debug, Clone)]
pub struct RegistrySnapshot {
    pub state: State,
    pub head: Head,
    pub root: [u8; 32],
}

pub async fn read_snapshot(
    vault_fs: &VaultFs,
    config: &VaultConfig,
) -> Result<Option<RegistrySnapshot>, SyncError> {
    let keys = ManifestKeys::derive(config)?;
    Ok(match read_log(vault_fs, config, &keys).await? {
        LogRead::V2(l) => Some(RegistrySnapshot {
            state: l.state,
            head: l.head,
            root: keys.vault_id,
        }),
        LogRead::Empty | LogRead::Legacy { .. } => None,
    })
}

pub const REGISTER_BATCH: usize = 1000;
pub const FLUSH_DELAY: Duration = Duration::from_secs(1);
pub const REVERIFY_AFTER: Duration = Duration::from_secs(30);

struct Waiter {
    ops: Vec<Op>,
    tx: oneshot::Sender<Result<(), Msg>>,
}

struct Core {
    heads: Db,
    base: Option<LogRead>,
    verified_at: Option<Instant>,
}

pub struct Registrar {
    me: Weak<Registrar>,
    fs: OnceLock<Weak<VaultFs>>,
    config: Mutex<VaultConfig>,
    keys: ManifestKeys,
    writer: Writer,
    core: tokio::sync::Mutex<Core>,
    queue: Mutex<Vec<Waiter>>,
    confirms: Mutex<Vec<Op>>,
    flush_scheduled: AtomicBool,
}

fn unreadable(e: &SyncError) -> Msg {
    if let SyncError::IntegrityFailure(msg) = e {
        return msg.clone();
    }
    Msg::new("REGISTRY_UNREADABLE").arg(e)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl Registrar {
    pub fn new(config: &VaultConfig, writer: Writer, heads: Db) -> Result<Arc<Self>, SyncError> {
        let keys = ManifestKeys::derive(config)?;
        Ok(Arc::new_cyclic(|me| Self {
            me: me.clone(),
            fs: OnceLock::new(),
            config: Mutex::new(config.clone()),
            keys,
            writer,
            core: tokio::sync::Mutex::new(Core {
                heads,
                base: None,
                verified_at: None,
            }),
            queue: Mutex::new(Vec::new()),
            confirms: Mutex::new(Vec::new()),
            flush_scheduled: AtomicBool::new(false),
        }))
    }

    pub fn bind(&self, fs: Weak<VaultFs>) {
        let _ = self.fs.set(fs);
    }

    pub fn set_config(&self, config: &VaultConfig) {
        *self.config.lock().expect("mutex poisoned") = config.clone();
    }

    pub async fn register(
        &self,
        entries: Vec<(EntryKey, Node)>,
        confirm: Vec<Op>,
    ) -> Result<(), Msg> {
        for chunk in entries.chunks(REGISTER_BATCH) {
            let puts: Vec<Op> = chunk
                .iter()
                .map(|(key, node)| Op::Put {
                    key: key.clone(),
                    node: node.clone(),
                })
                .collect();
            self.begin(&puts).await.map_err(|e| e.msg())?;
            self.end(&puts, true).await;
        }
        if !confirm.is_empty() {
            self.confirms
                .lock()
                .expect("mutex poisoned")
                .extend(confirm);
        }
        self.flush().await
    }

    pub async fn seal(&self) -> Result<(), Msg> {
        let mut core = self.core.lock().await;
        let ops = std::mem::take(&mut *self.confirms.lock().expect("mutex poisoned"));
        let result = self.append_forced(&mut core, &ops, Some(true)).await;
        if result.is_err() {
            let mut confirms = self.confirms.lock().expect("mutex poisoned");
            let newer = std::mem::take(&mut *confirms);
            *confirms = ops;
            confirms.extend(newer);
        }
        result
    }

    pub async fn reset(&self) -> Result<(), Msg> {
        let fs = self
            .fs
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Msg::new("REGISTRY_VAULT_CLOSED"))?;
        let config = self.config.lock().expect("mutex poisoned").clone();
        let mut core = self.core.lock().await;
        let dir = fs
            .resolve_or_create_dir_ref_untracked(&config, cu_format::manifest::MANIFEST_DIR)
            .await
            .map_err(|e| e.msg())?;
        let names: Vec<String> = fs
            .list_in(&config, &dir)
            .await
            .map_err(|e| e.msg())?
            .into_iter()
            .map(|e| e.name)
            .collect();
        let highest_listed = names
            .iter()
            .filter_map(|n| cu_format::manifest::manifest_seq_from_name(n))
            .max()
            .unwrap_or(0);
        let remembered = core
            .heads
            .get_manifest_head(&self.keys.vault_id)
            .map_err(|e| e.msg())?
            .map_or(0, |h| h.seq);
        let next_base = highest_listed.max(remembered);
        for name in names {
            let _ = fs.remove_file_in(&config, &dir, &name).await;
        }
        let ctx = LogCtx {
            vault_fs: &fs,
            config: &config,
            keys: &self.keys,
        };
        let mut stores = HeadStores {
            job: None,
            shared: Some(&mut core.heads),
        };
        let base = LogRead::Legacy {
            seq: next_base,
            payload_hash: [0u8; 32],
        };
        let result = append_ops(
            &ctx,
            &mut stores,
            &self.writer,
            &[],
            now_ms(),
            base,
            Some(false),
        )
        .await;
        match result {
            Ok(loaded) => {
                core.base = Some(LogRead::V2(Box::new(loaded)));
                core.verified_at = Some(Instant::now());
                Ok(())
            }
            Err(e) => {
                core.base = None;
                fs.clear_internal_dir_cache();
                Err(e.msg())
            }
        }
    }

    pub async fn flush(&self) -> Result<(), Msg> {
        let mut core = self.core.lock().await;
        let ops = std::mem::take(&mut *self.confirms.lock().expect("mutex poisoned"));
        if ops.is_empty() {
            return Ok(());
        }
        let result = self.append(&mut core, &ops).await;
        if result.is_err() {
            let mut confirms = self.confirms.lock().expect("mutex poisoned");
            let newer = std::mem::take(&mut *confirms);
            *confirms = ops;
            confirms.extend(newer);
        }
        result
    }

    async fn append(&self, core: &mut Core, ops: &[Op]) -> Result<(), Msg> {
        self.append_forced(core, ops, None).await
    }

    async fn append_forced(
        &self,
        core: &mut Core,
        ops: &[Op],
        force_coverage: Option<bool>,
    ) -> Result<(), Msg> {
        let fs = self
            .fs
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Msg::new("REGISTRY_CLOSED_BEFORE_RECORD"))?;
        let config = self.config.lock().expect("mutex poisoned").clone();
        let ctx = LogCtx {
            vault_fs: &fs,
            config: &config,
            keys: &self.keys,
        };
        let Core {
            heads,
            base,
            verified_at,
        } = core;
        let mut stores = HeadStores {
            job: None,
            shared: Some(heads),
        };
        let fresh = verified_at.is_some_and(|t| t.elapsed() < REVERIFY_AFTER);
        let current = match base.take() {
            Some(b) if fresh => b,
            _ => {
                let read = read_log(&fs, &config, &self.keys)
                    .await
                    .map_err(|e| unreadable(&e))?;
                check_head(&stores, &self.keys.vault_id, &read).map_err(|e| e.msg())?;
                *verified_at = Some(Instant::now());
                read
            }
        };
        match append_ops(
            &ctx,
            &mut stores,
            &self.writer,
            ops,
            now_ms(),
            current,
            force_coverage,
        )
        .await
        {
            Ok(loaded) => {
                *base = Some(LogRead::V2(Box::new(loaded)));
                Ok(())
            }
            Err(e) => {
                *base = None;
                fs.clear_internal_dir_cache();
                Err(e.msg())
            }
        }
    }

    async fn refresh(&self, core: &mut Core) -> Result<(), Msg> {
        let fs = self
            .fs
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Msg::new("REGISTRY_VAULT_CLOSED"))?;
        let config = self.config.lock().expect("mutex poisoned").clone();
        let read = read_log(&fs, &config, &self.keys)
            .await
            .map_err(|e| unreadable(&e))?;
        let stores = HeadStores {
            job: None,
            shared: Some(&mut core.heads),
        };
        check_head(&stores, &self.keys.vault_id, &read).map_err(|e| e.msg())?;
        core.base = Some(read);
        core.verified_at = Some(Instant::now());
        Ok(())
    }

    async fn judge(&self, key: &EntryKey, served: &Node) -> Result<(), VfsError> {
        let mut core = self.core.lock().await;
        if core.base.is_none() {
            self.refresh(&mut core).await.map_err(VfsError::Registry)?;
        }
        if Self::refused(&core, key, served).is_none() {
            return Ok(());
        }
        self.refresh(&mut core).await.map_err(VfsError::Registry)?;
        match Self::refused(&core, key, served) {
            None => Ok(()),
            Some(why) => Err(VfsError::Registry(why)),
        }
    }

    fn refused(core: &Core, key: &EntryKey, served: &Node) -> Option<Msg> {
        let sealed = matches!(&core.base, Some(LogRead::V2(l)) if l.state.coverage);
        match Self::verdict(core, key, served) {
            Acceptance::Mismatch => Some(Msg::new("REGISTRY_SERVED_DIFFERENT").arg(&key.name)),
            Acceptance::Unknown if sealed => {
                Some(Msg::new("REGISTRY_NOT_REGISTERED").arg(&key.name))
            }
            Acceptance::Current | Acceptance::Pending | Acceptance::Unknown => None,
        }
    }

    fn verdict(core: &Core, key: &EntryKey, served: &Node) -> Acceptance {
        match &core.base {
            Some(LogRead::V2(l)) => l.state.judge(key, Some(served)),
            _ => Acceptance::Unknown,
        }
    }

    async fn drain(&self) {
        let mut core = self.core.lock().await;
        let waiters = std::mem::take(&mut *self.queue.lock().expect("mutex poisoned"));
        if waiters.is_empty() {
            return;
        }
        let confirms = std::mem::take(&mut *self.confirms.lock().expect("mutex poisoned"));
        let mut ops = confirms.clone();
        for w in &waiters {
            ops.extend(w.ops.iter().cloned());
        }
        let result = self.append(&mut core, &ops).await;
        if result.is_err() && !confirms.is_empty() {
            let mut queued = self.confirms.lock().expect("mutex poisoned");
            let newer = std::mem::take(&mut *queued);
            *queued = confirms;
            queued.extend(newer);
        }
        for w in waiters {
            let _ = w.tx.send(result.clone());
        }
    }

    fn schedule_flush(&self) {
        if self.flush_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(me) = self.me.upgrade() else {
            self.flush_scheduled.store(false, Ordering::SeqCst);
            return;
        };
        tokio::spawn(async move {
            tokio::time::sleep(FLUSH_DELAY).await;
            me.flush_scheduled.store(false, Ordering::SeqCst);
            let _ = me.flush().await;
        });
    }
}

#[async_trait]
impl NamespaceObserver for Registrar {
    async fn begin(&self, ops: &[Op]) -> Result<(), VfsError> {
        let (tx, rx) = oneshot::channel();
        self.queue.lock().expect("mutex poisoned").push(Waiter {
            ops: ops.to_vec(),
            tx,
        });
        self.drain().await;
        match rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(msg)) => Err(VfsError::Registry(msg)),
            Err(_) => Err(VfsError::Registry(Msg::new("REGISTRY_RECORDER_STOPPED"))),
        }
    }

    async fn check_file(&self, key: &EntryKey, served: &[u8; NONCE_LEN]) -> Result<(), VfsError> {
        self.judge(key, &Node::File { nonce: *served }).await
    }

    async fn check_dir(&self, key: &EntryKey, served: &[u8; 32]) -> Result<(), VfsError> {
        self.judge(key, &Node::Dir { dir_id: *served }).await
    }

    async fn end(&self, ops: &[Op], committed: bool) {
        let derived: Vec<Op> = ops
            .iter()
            .filter_map(|op| {
                if committed {
                    op.confirmation()
                } else {
                    op.abortion()
                }
            })
            .collect();
        if derived.is_empty() {
            return;
        }
        self.confirms
            .lock()
            .expect("mutex poisoned")
            .extend(derived);
        self.schedule_flush();
    }
}
