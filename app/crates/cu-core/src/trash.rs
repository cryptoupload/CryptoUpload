// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::vault::{NodeKind, Vault};

const TRASH_DIR: &str = ".cryptoupload/trash";
const INDEX_PATH: &str = ".cryptoupload/trash/index.json";
const INDEX_FILE_NAME: &str = "index.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrashKind {
    File,
    Dir,
}

impl From<NodeKind> for TrashKind {
    fn from(kind: NodeKind) -> Self {
        match kind {
            NodeKind::File => Self::File,
            NodeKind::Dir => Self::Dir,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashEntry {
    pub id: String,
    pub original_path: Option<String>,
    pub name: String,
    pub kind: TrashKind,
    pub deleted_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrashIndex {
    entries: Vec<TrashEntry>,
}

impl Vault {
    pub async fn trash(&self, vault_path: &str) -> Result<(), CoreError> {
        let result = async {
            let (parent, name) = split_parent_and_name(vault_path);
            let kind = self.entry_kind(&parent, &name).await?;
            let mut index = self.read_index().await?;

            self.mkdir(TRASH_DIR).await?;
            let id = generate_id()?;
            let dest = format!("{TRASH_DIR}/{id}");
            self.fs().rename(self.config(), vault_path, &dest).await?;

            index.entries.push(TrashEntry {
                id,
                original_path: Some(vault_path.trim_matches('/').to_string()),
                name,
                kind: kind.into(),
                deleted_at: now_secs(),
            });
            let _ = self.write_index(&index).await;
            Ok(())
        }
        .await;
        self.settled(result).await
    }

    pub async fn list_trash(&self) -> Result<Vec<TrashEntry>, CoreError> {
        let index = self.read_index().await?;
        let mut entries = index.entries;

        let physical = match self.list(TRASH_DIR).await {
            Ok(list) => list,
            Err(CoreError::NotFound) => Vec::new(),
            Err(e) => return Err(e),
        };
        for phys in physical {
            if phys.name == INDEX_FILE_NAME {
                continue;
            }
            if entries.iter().any(|e| e.id == phys.name) {
                continue;
            }
            entries.push(TrashEntry {
                id: phys.name.clone(),
                original_path: None,
                name: phys.name,
                kind: phys.kind.into(),
                deleted_at: 0,
            });
        }
        entries.sort_by_key(|e| std::cmp::Reverse(e.deleted_at));
        Ok(entries)
    }

    pub async fn restore_from_trash(&self, id: &str) -> Result<String, CoreError> {
        let result = async {
            let mut index = self.read_index().await?;
            let pos = index.entries.iter().position(|e| e.id == id);
            let (original_path, name) = if let Some(i) = &pos {
                let e = &index.entries[*i];
                (e.original_path.clone(), e.name.clone())
            } else {
                self.entry_kind(TRASH_DIR, id).await?;
                (None, id.to_string())
            };

            let base_target = original_path.clone().unwrap_or_else(|| name.clone());
            if let Some((parent, _)) = base_target.rsplit_once('/') {
                self.mkdir(parent).await?;
            }
            let final_target = self.first_free_path(&base_target).await?;

            let src = format!("{TRASH_DIR}/{id}");
            self.fs().rename(self.config(), &src, &final_target).await?;

            if let Some(i) = pos {
                index.entries.remove(i);
                let _ = self.write_index(&index).await;
            }
            Ok(final_target)
        }
        .await;
        self.settled(result).await
    }

    pub async fn purge_trash(&self, id: &str) -> Result<(), CoreError> {
        let result = self.purge_trash_inner(id).await;
        self.settled(result).await
    }

    async fn purge_trash_inner(&self, id: &str) -> Result<(), CoreError> {
        let mut index = self.read_index().await?;
        let pos = index.entries.iter().position(|e| e.id == id);
        let kind: TrashKind = match pos {
            Some(i) => index.entries[i].kind,
            None => self.entry_kind(TRASH_DIR, id).await?.into(),
        };

        let path = format!("{TRASH_DIR}/{id}");
        match kind {
            TrashKind::File => self.fs().remove_file(self.config(), &path).await?,
            TrashKind::Dir => self.fs().remove_dir(self.config(), &path).await?,
        }

        if let Some(i) = pos {
            index.entries.remove(i);
            let _ = self.write_index(&index).await;
        }
        Ok(())
    }

    pub async fn empty_trash(&self) -> Result<(), CoreError> {
        let result = async {
            for entry in self.list_trash().await? {
                self.purge_trash_inner(&entry.id).await?;
            }
            let _ = self.write_index(&TrashIndex::default()).await;
            Ok(())
        }
        .await;
        self.settled(result).await
    }

    async fn entry_kind(&self, parent: &str, name: &str) -> Result<NodeKind, CoreError> {
        let entries = self.list(parent).await?;
        entries
            .into_iter()
            .find(|e| cu_format::name::nfc(&e.name) == cu_format::name::nfc(name))
            .map(|e| e.kind)
            .ok_or(CoreError::NotFound)
    }

    async fn first_free_path(&self, path: &str) -> Result<String, CoreError> {
        if !self.fs().exists(self.config(), path).await? {
            return Ok(path.to_string());
        }
        let (parent, name) = split_parent_and_name(path);
        let (stem, ext) = split_stem_ext(&name);
        let mut attempt: u32 = 1;
        loop {
            let suffix = if attempt == 1 {
                " (restored)".to_string()
            } else {
                format!(" (restored {attempt})")
            };
            let candidate_name = format!("{stem}{suffix}{ext}");
            let candidate = if parent.is_empty() {
                candidate_name
            } else {
                format!("{parent}/{candidate_name}")
            };
            if !self.fs().exists(self.config(), &candidate).await? {
                return Ok(candidate);
            }
            attempt += 1;
        }
    }

    async fn read_index(&self) -> Result<TrashIndex, CoreError> {
        let mut buf = Vec::new();
        match self
            .fs()
            .read_file(self.config(), INDEX_PATH, &mut buf, &mut |_| {})
            .await
        {
            Ok(()) => Ok(serde_json::from_slice(&buf).unwrap_or_default()),
            Err(cu_vfs::VfsError::NotFound) => Ok(TrashIndex::default()),
            Err(e) => Err(e.into()),
        }
    }

    async fn write_index(&self, index: &TrashIndex) -> Result<(), CoreError> {
        let bytes = serde_json::to_vec_pretty(index).unwrap_or_default();
        let mut cursor = std::io::Cursor::new(bytes);
        self.fs()
            .write_file(self.config(), INDEX_PATH, &mut cursor, None, &mut |_| {})
            .await?;
        Ok(())
    }
}

fn split_parent_and_name(path: &str) -> (String, String) {
    let trimmed = path.trim_matches('/');
    match trimmed.rsplit_once('/') {
        Some((parent, name)) => (parent.to_string(), name.to_string()),
        None => (String::new(), trimmed.to_string()),
    }
}

fn split_stem_ext(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
        _ => (name.to_string(), String::new()),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn generate_id() -> Result<String, CoreError> {
    let mut buf = [0u8; 6];
    cu_crypto::random::fill_random(&mut buf).map_err(|_| CoreError::RandomUnavailable)?;
    Ok(format!("{}-{}", now_secs(), hex(&buf)))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}
