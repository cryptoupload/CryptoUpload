// SPDX-License-Identifier: GPL-3.0-or-later

pub mod error;
pub mod schema;

pub use error::IndexError;

use std::path::Path;
use std::sync::{Arc, LazyLock};

use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

pub type RootId = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub [u8; 16]);

impl NodeId {
    #[must_use]
    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
}

impl NodeKind {
    const fn to_i64(self) -> i64 {
        match self {
            Self::File => 0,
            Self::Dir => 1,
        }
    }

    const fn from_i64(v: i64) -> Self {
        if v == 1 { Self::Dir } else { Self::File }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanFingerprint {
    pub dev: u64,
    pub ino: u64,
    pub size: u64,
    pub mtime_ns: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NormalizeReport {
    pub renamed: u64,
    pub forgotten: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRow {
    pub node_id: NodeId,
    pub root_id: RootId,
    pub parent_id: Option<NodeId>,
    pub name: String,
    pub kind: NodeKind,
    pub size: u64,
    pub mtime_ns: i64,
    pub content_hash: Option<[u8; 32]>,
    pub version: i64,
    pub scan: Option<ScanFingerprint>,
    pub header_nonce: Option<[u8; HEADER_NONCE_LEN]>,
}

pub const HEADER_NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionLocation {
    Canonical,
    Archived,
}

impl VersionLocation {
    const fn to_i64(self) -> i64 {
        match self {
            Self::Canonical => 0,
            Self::Archived => 1,
        }
    }

    const fn from_i64(v: i64) -> Self {
        if v == 1 {
            Self::Archived
        } else {
            Self::Canonical
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRow {
    pub node_id: NodeId,
    pub version: i64,
    pub created_at: i64,
    pub kind: NodeKind,
    pub parent_id: Option<NodeId>,
    pub name: String,
    pub size: u64,
    pub content_hash: Option<[u8; 32]>,
    pub location: VersionLocation,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalOp {
    Mkdir,
    Upload,
    Archive,
    Rename,
    Delete,
    Prune,
    SetAside,
}

impl JournalOp {
    const fn to_i64(self) -> i64 {
        match self {
            Self::Mkdir => 0,
            Self::Upload => 1,
            Self::Archive => 2,
            Self::Rename => 3,
            Self::Delete => 4,
            Self::Prune => 5,
            Self::SetAside => 6,
        }
    }

    const fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Upload,
            2 => Self::Archive,
            3 => Self::Rename,
            4 => Self::Delete,
            5 => Self::Prune,
            6 => Self::SetAside,
            _ => Self::Mkdir,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalState {
    Planned,
    Started,
    RemoteDone,
    Committed,
}

impl JournalState {
    const fn to_i64(self) -> i64 {
        match self {
            Self::Planned => 0,
            Self::Started => 1,
            Self::RemoteDone => 2,
            Self::Committed => 3,
        }
    }

    const fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Started,
            2 => Self::RemoteDone,
            3 => Self::Committed,
            _ => Self::Planned,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntryRow {
    pub entry_id: i64,
    pub run_id: i64,
    pub root_id: RootId,
    pub node_id: NodeId,
    pub seq: i64,
    pub op: JournalOp,
    pub state: JournalState,
    pub payload: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestHeadRow {
    pub vault_id: [u8; 32],
    pub seq: i64,
    pub payload_hash: [u8; 32],
    pub mac: Vec<u8>,
    pub device_id: [u8; 16],
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Ok,
    Failed,
    Interrupted,
}

impl RunOutcome {
    const fn to_i64(self) -> i64 {
        match self {
            Self::Ok => 0,
            Self::Failed => 1,
            Self::Interrupted => 2,
        }
    }

    const fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Failed,
            2 => Self::Interrupted,
            _ => Self::Ok,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub run_id: i64,
    pub root_id: RootId,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub outcome: Option<RunOutcome>,
    pub files_uploaded: u64,
    pub files_deleted: u64,
    pub files_renamed: u64,
    pub files_modified: Option<u64>,
    pub bytes_transferred: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUploadRow {
    pub upload_row_id: i64,
    pub remote_id: String,
    pub object_key: String,
    pub upload_id: Option<String>,
    pub part_size: u64,
    pub prologue: Option<Vec<u8>>,
    pub source_hash: [u8; 32],
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUploadPartRow {
    pub part_number: u32,
    pub etag: String,
    pub crc32c: [u8; 4],
    pub len: u64,
    pub uploaded_at: i64,
}

pub struct Db {
    conn: Connection,
    sealed: std::cell::Cell<bool>,
    cipher: Option<Arc<dyn FieldCipher>>,
}

pub trait FieldCipher: Send + Sync {
    fn seal(&self, plain: &str) -> Option<String>;
    fn open(&self, stored: &str) -> Option<String>;
}

const NODE_COLUMNS: &str = "node_id, root_id, parent_id, name, kind, size, mtime_ns, \
     content_hash, version, scan_dev, scan_ino, scan_size, scan_mtime_ns, header_nonce";

static SQL_INSERT_NODE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "INSERT INTO node ({NODE_COLUMNS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)"
    )
});
static SQL_GET_NODE: LazyLock<String> =
    LazyLock::new(|| format!("SELECT {NODE_COLUMNS} FROM node WHERE node_id = ?1"));
static SQL_FIND_CHILD: LazyLock<String> = LazyLock::new(|| {
    format!("SELECT {NODE_COLUMNS} FROM node WHERE root_id = ?1 AND parent_id IS ?2 AND name = ?3")
});
static SQL_LIST_CHILDREN: LazyLock<String> = LazyLock::new(|| {
    format!("SELECT {NODE_COLUMNS} FROM node WHERE root_id = ?1 AND parent_id IS ?2")
});
static SQL_LIST_ALL_NODES: LazyLock<String> =
    LazyLock::new(|| format!("SELECT {NODE_COLUMNS} FROM node WHERE root_id = ?1"));

fn row_to_node(row: &Row<'_>) -> rusqlite::Result<NodeRow> {
    let node_id: Vec<u8> = row.get(0)?;
    let parent_id: Option<Vec<u8>> = row.get(2)?;
    let content_hash: Option<Vec<u8>> = row.get(7)?;
    let scan_dev: Option<i64> = row.get(9)?;
    let scan_ino: Option<i64> = row.get(10)?;
    let scan_size: Option<i64> = row.get(11)?;
    let scan_mtime_ns: Option<i64> = row.get(12)?;
    let header_nonce: Option<Vec<u8>> = row.get(13)?;
    Ok(NodeRow {
        node_id: NodeId(to_array16(&node_id)),
        root_id: row.get(1)?,
        parent_id: parent_id.map(|b| NodeId(to_array16(&b))),
        name: row.get(3)?,
        kind: NodeKind::from_i64(row.get(4)?),
        #[allow(clippy::cast_sign_loss)]
        size: row.get::<_, i64>(5)? as u64,
        mtime_ns: row.get(6)?,
        content_hash: content_hash.map(|b| to_array32(&b)),
        version: row.get(8)?,
        scan: match (scan_dev, scan_ino, scan_size, scan_mtime_ns) {
            #[allow(clippy::cast_sign_loss)]
            (Some(dev), Some(ino), Some(size), Some(mtime_ns)) => Some(ScanFingerprint {
                dev: dev as u64,
                ino: ino as u64,
                size: size as u64,
                mtime_ns,
            }),
            _ => None,
        },
        header_nonce: header_nonce.and_then(|b| b.as_slice().try_into().ok()),
    })
}

#[allow(clippy::cast_possible_wrap)]
const fn u64_to_sql(v: u64) -> i64 {
    v as i64
}

fn to_array16(bytes: &[u8]) -> [u8; 16] {
    bytes
        .try_into()
        .expect("node_id in state.db is not 16 bytes: corrupt file")
}

fn to_array32(bytes: &[u8]) -> [u8; 32] {
    bytes
        .try_into()
        .expect("hash in state.db is not 32 bytes: corrupt file")
}

fn to_array4(bytes: &[u8]) -> [u8; 4] {
    bytes
        .try_into()
        .expect("crc32c in state.db is not 4 bytes: corrupt file")
}

const REMOTE_UPLOAD_COLUMNS: &str = "upload_row_id, remote_id, object_key, upload_id, \
     part_size, prologue, source_hash, created_at, updated_at";

fn row_to_remote_upload(row: &Row<'_>) -> rusqlite::Result<RemoteUploadRow> {
    let source_hash: Vec<u8> = row.get(6)?;
    Ok(RemoteUploadRow {
        upload_row_id: row.get(0)?,
        remote_id: row.get(1)?,
        object_key: row.get(2)?,
        upload_id: row.get(3)?,
        #[allow(clippy::cast_sign_loss)]
        part_size: row.get::<_, i64>(4)? as u64,
        prologue: row.get(5)?,
        source_hash: to_array32(&source_hash),
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn row_to_remote_upload_part(row: &Row<'_>) -> rusqlite::Result<RemoteUploadPartRow> {
    let crc32c: Vec<u8> = row.get(2)?;
    Ok(RemoteUploadPartRow {
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        part_number: row.get::<_, i64>(0)? as u32,
        etag: row.get(1)?,
        crc32c: to_array4(&crc32c),
        #[allow(clippy::cast_sign_loss)]
        len: row.get::<_, i64>(3)? as u64,
        uploaded_at: row.get(4)?,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, IndexError> {
        Self::open_inner(path, None)
    }

    pub fn open_with_cipher(
        path: &Path,
        cipher: Arc<dyn FieldCipher>,
        sealed: bool,
    ) -> Result<Self, IndexError> {
        let db = Self::open_inner(path, Some(cipher))?;
        db.set_sealed(sealed)?;
        Ok(db)
    }

    pub fn open_reading(
        path: &Path,
        cipher: Option<Arc<dyn FieldCipher>>,
    ) -> Result<Self, IndexError> {
        Self::open_inner(path, cipher)
    }

    fn open_inner(path: &Path, cipher: Option<Arc<dyn FieldCipher>>) -> Result<Self, IndexError> {
        let conn = Connection::open(path)?;
        restrict_permissions(path);
        let mut db = Self::init(conn, true)?;
        db.cipher = cipher;
        for sidecar in Self::wal_sidecar_paths(path) {
            restrict_permissions(&sidecar);
        }
        Ok(db)
    }

    fn init(conn: Connection, wal: bool) -> Result<Self, IndexError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        if wal {
            conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "cache_size", -16384)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&conn)?;
        let sealed: String =
            conn.query_row("SELECT value FROM meta WHERE key = 'sealed'", [], |r| {
                r.get(0)
            })?;
        Ok(Self {
            conn,
            sealed: std::cell::Cell::new(sealed == "1"),
            cipher: None,
        })
    }

    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.sealed.get()
    }

    fn enc<'a>(&self, plain: &'a str) -> Result<std::borrow::Cow<'a, str>, IndexError> {
        if !self.sealed.get() {
            return Ok(std::borrow::Cow::Borrowed(plain));
        }
        let cipher = self.cipher.as_ref().ok_or(IndexError::Locked)?;
        cipher
            .seal(plain)
            .map(std::borrow::Cow::Owned)
            .ok_or(IndexError::SealFailed)
    }

    fn dec(&self, stored: String) -> Result<String, IndexError> {
        if !self.sealed.get() {
            return Ok(stored);
        }
        let cipher = self.cipher.as_ref().ok_or(IndexError::Locked)?;
        cipher.open(&stored).ok_or(IndexError::SealBroken)
    }

    fn dec_node(&self, mut node: NodeRow) -> Result<NodeRow, IndexError> {
        node.name = self.dec(node.name)?;
        Ok(node)
    }

    fn dec_nodes(&self, nodes: Vec<NodeRow>) -> Result<Vec<NodeRow>, IndexError> {
        nodes.into_iter().map(|node| self.dec_node(node)).collect()
    }

    fn dec_versions(&self, versions: Vec<VersionRow>) -> Result<Vec<VersionRow>, IndexError> {
        versions
            .into_iter()
            .map(|mut version| {
                version.name = self.dec(version.name)?;
                Ok(version)
            })
            .collect()
    }

    pub fn set_sealed(&self, sealed: bool) -> Result<(), IndexError> {
        if self.sealed.get() == sealed {
            return Ok(());
        }
        let cipher = Arc::clone(self.cipher.as_ref().ok_or(IndexError::Locked)?);
        let convert = |text: String| -> Result<String, IndexError> {
            if sealed {
                cipher.seal(&text).ok_or(IndexError::SealFailed)
            } else {
                cipher.open(&text).ok_or(IndexError::SealBroken)
            }
        };
        self.in_transaction(|| {
            for (table, column) in [
                ("node", "name"),
                ("node_version", "name"),
                ("journal", "payload"),
            ] {
                let mut select = self
                    .conn
                    .prepare(&format!("SELECT rowid, {column} FROM {table}"))?;
                let rows: Vec<(i64, String)> = select
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                drop(select);
                let mut update = self.conn.prepare(&format!(
                    "UPDATE {table} SET {column} = ?2 WHERE rowid = ?1"
                ))?;
                for (rowid, text) in rows {
                    update.execute(params![rowid, convert(text)?])?;
                }
            }
            self.conn.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'sealed'",
                params![if sealed { "1" } else { "0" }],
            )?;
            Ok(())
        })?;
        self.sealed.set(sealed);
        if sealed {
            self.conn.execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
            )?;
        }
        Ok(())
    }

    fn wal_sidecar_paths(path: &Path) -> [std::path::PathBuf; 2] {
        let mut wal = path.as_os_str().to_owned();
        wal.push("-wal");
        let mut shm = path.as_os_str().to_owned();
        shm.push("-shm");
        [std::path::PathBuf::from(wal), std::path::PathBuf::from(shm)]
    }

    pub fn begin_transaction(&self) -> Result<(), IndexError> {
        self.conn.execute_batch("BEGIN")?;
        Ok(())
    }

    pub fn commit_transaction(&self) -> Result<(), IndexError> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    pub fn rollback_transaction(&self) -> Result<(), IndexError> {
        self.conn.execute_batch("ROLLBACK")?;
        Ok(())
    }

    pub fn in_transaction<T>(
        &self,
        body: impl FnOnce() -> Result<T, IndexError>,
    ) -> Result<T, IndexError> {
        self.begin_transaction()?;
        match body() {
            Ok(v) => {
                self.commit_transaction()?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.rollback_transaction();
                Err(e)
            }
        }
    }

    pub fn find_root(
        &self,
        local_path: &str,
        vault_path: &str,
    ) -> Result<Option<RootId>, IndexError> {
        Ok(self
            .conn
            .query_row(
                "SELECT root_id FROM root WHERE local_path = ?1 AND vault_path = ?2",
                params![local_path, vault_path],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn get_or_create_root(
        &self,
        local_path: &str,
        vault_path: &str,
        created_at_ms: i64,
    ) -> Result<RootId, IndexError> {
        self.conn.execute(
            "INSERT INTO root (local_path, vault_path, created_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (local_path, vault_path) DO NOTHING",
            params![local_path, vault_path, created_at_ms],
        )?;
        self.find_root(local_path, vault_path)?
            .ok_or(IndexError::Sqlite(rusqlite::Error::QueryReturnedNoRows))
    }

    pub fn insert_node(&self, node: &NodeRow) -> Result<(), IndexError> {
        self.conn
            .prepare_cached(&SQL_INSERT_NODE)?
            .execute(params![
                node.node_id.as_slice(),
                node.root_id,
                node.parent_id.map(|p| p.0.to_vec()),
                self.enc(&node.name)?,
                node.kind.to_i64(),
                u64_to_sql(node.size),
                node.mtime_ns,
                node.content_hash.map(|h| h.to_vec()),
                node.version,
                node.scan.map(|s| u64_to_sql(s.dev)),
                node.scan.map(|s| u64_to_sql(s.ino)),
                node.scan.map(|s| u64_to_sql(s.size)),
                node.scan.map(|s| s.mtime_ns),
                node.header_nonce.map(|n| n.to_vec()),
            ])?;
        Ok(())
    }

    pub fn get_node(&self, node_id: NodeId) -> Result<Option<NodeRow>, IndexError> {
        self.conn
            .prepare_cached(&SQL_GET_NODE)?
            .query_row(params![node_id.as_slice()], row_to_node)
            .optional()?
            .map(|node| self.dec_node(node))
            .transpose()
    }

    pub fn find_child(
        &self,
        root_id: RootId,
        parent_id: Option<NodeId>,
        name: &str,
    ) -> Result<Option<NodeRow>, IndexError> {
        self.conn
            .prepare_cached(&SQL_FIND_CHILD)?
            .query_row(
                params![root_id, parent_id.map(|p| p.0.to_vec()), self.enc(name)?],
                row_to_node,
            )
            .optional()?
            .map(|node| self.dec_node(node))
            .transpose()
    }

    pub fn list_children(
        &self,
        root_id: RootId,
        parent_id: Option<NodeId>,
    ) -> Result<Vec<NodeRow>, IndexError> {
        let mut stmt = self.conn.prepare_cached(&SQL_LIST_CHILDREN)?;
        let rows = stmt.query_map(
            params![root_id, parent_id.map(|p| p.0.to_vec())],
            row_to_node,
        )?;
        self.dec_nodes(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn list_all_nodes(&self, root_id: RootId) -> Result<Vec<NodeRow>, IndexError> {
        let mut stmt = self.conn.prepare_cached(&SQL_LIST_ALL_NODES)?;
        let rows = stmt.query_map(params![root_id], row_to_node)?;
        self.dec_nodes(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn recent_files(&self, root_id: RootId, limit: u32) -> Result<Vec<NodeRow>, IndexError> {
        let sql = format!(
            "SELECT {NODE_COLUMNS} FROM node WHERE root_id = ?1 AND kind = ?2 \
             ORDER BY mtime_ns DESC LIMIT ?3"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(
            params![root_id, NodeKind::File.to_i64(), limit],
            row_to_node,
        )?;
        self.dec_nodes(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn root_file_totals(&self, root_id: RootId) -> Result<(u64, u64), IndexError> {
        let (count, bytes): (i64, i64) = self.conn.prepare_cached(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM node WHERE root_id = ?1 AND kind = ?2",
        )?
        .query_row(params![root_id, NodeKind::File.to_i64()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        Ok((
            u64::try_from(count).unwrap_or(0),
            u64::try_from(bytes).unwrap_or(0),
        ))
    }

    pub fn rename_node(
        &self,
        node_id: NodeId,
        new_parent: Option<NodeId>,
        new_name: &str,
    ) -> Result<(), IndexError> {
        self.conn
            .prepare_cached("UPDATE node SET parent_id = ?2, name = ?3 WHERE node_id = ?1")?
            .execute(params![
                node_id.as_slice(),
                new_parent.map(|p| p.0.to_vec()),
                self.enc(new_name)?
            ])?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_baseline(
        &self,
        node_id: NodeId,
        size: u64,
        mtime_ns: i64,
        content_hash: Option<[u8; 32]>,
        version: i64,
        scan: Option<ScanFingerprint>,
        header_nonce: Option<[u8; HEADER_NONCE_LEN]>,
    ) -> Result<(), IndexError> {
        self.conn
            .prepare_cached(
                "UPDATE node SET size = ?2, mtime_ns = ?3, content_hash = ?4, version = ?5, \
                 scan_dev = ?6, scan_ino = ?7, scan_size = ?8, scan_mtime_ns = ?9, \
                 header_nonce = ?10 WHERE node_id = ?1",
            )?
            .execute(params![
                node_id.as_slice(),
                u64_to_sql(size),
                mtime_ns,
                content_hash.map(|h| h.to_vec()),
                version,
                scan.map(|s| u64_to_sql(s.dev)),
                scan.map(|s| u64_to_sql(s.ino)),
                scan.map(|s| u64_to_sql(s.size)),
                scan.map(|s| s.mtime_ns),
                header_nonce.map(|n| n.to_vec()),
            ])?;
        Ok(())
    }

    pub fn delete_node(&self, node_id: NodeId) -> Result<(), IndexError> {
        self.conn
            .prepare_cached("DELETE FROM node WHERE node_id = ?1")?
            .execute(params![node_id.as_slice()])?;
        Ok(())
    }

    pub fn names_normalized(&self, root_id: RootId) -> Result<bool, IndexError> {
        let flag: i64 = self.conn.query_row(
            "SELECT names_nfc FROM root WHERE root_id = ?1",
            params![root_id],
            |r| r.get(0),
        )?;
        Ok(flag != 0)
    }

    pub fn normalize_names(
        &self,
        root_id: RootId,
        normalize: &dyn Fn(&str) -> Option<String>,
    ) -> Result<NormalizeReport, IndexError> {
        self.in_transaction(|| {
            let mut report = NormalizeReport::default();
            for node in self.list_all_nodes(root_id)? {
                if node.name.is_ascii() {
                    continue;
                }
                let Some(new_name) = normalize(&node.name) else {
                    continue;
                };
                if self
                    .find_child(root_id, node.parent_id, &new_name)?
                    .is_some()
                {
                    self.delete_node(node.node_id)?;
                    report.forgotten += 1;
                } else {
                    self.rename_node(node.node_id, node.parent_id, &new_name)?;
                    report.renamed += 1;
                }
            }
            let mut stmt = self.conn.prepare(if self.sealed.get() {
                "SELECT node_id, version, name FROM node_version"
            } else {
                "SELECT node_id, version, name FROM node_version WHERE name GLOB '*[^ -~]*'"
            })?;
            let versions: Vec<(Vec<u8>, i64, String)> = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?;
            drop(stmt);
            for (node_id, version, name) in versions {
                let name = self.dec(name)?;
                if name.is_ascii() {
                    continue;
                }
                if let Some(new_name) = normalize(&name) {
                    self.conn.execute(
                        "UPDATE node_version SET name = ?3 WHERE node_id = ?1 AND version = ?2",
                        params![node_id, version, self.enc(&new_name)?],
                    )?;
                }
            }
            self.conn.execute(
                "UPDATE root SET names_nfc = 1 WHERE root_id = ?1",
                params![root_id],
            )?;
            Ok(report)
        })
    }

    pub fn insert_version(&self, v: &VersionRow) -> Result<(), IndexError> {
        self.conn
            .prepare_cached(
                "INSERT INTO node_version \
                 (node_id, version, created_at, kind, parent_id, name, size, content_hash, location, deleted) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            )?
            .execute(params![
                v.node_id.as_slice(),
                v.version,
                v.created_at,
                v.kind.to_i64(),
                v.parent_id.map(|p| p.0.to_vec()),
                self.enc(&v.name)?,
                u64_to_sql(v.size),
                v.content_hash.map(|h| h.to_vec()),
                v.location.to_i64(),
                i64::from(v.deleted),
            ])?;
        Ok(())
    }

    pub fn list_versions(&self, node_id: NodeId) -> Result<Vec<VersionRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT node_id, version, created_at, kind, parent_id, name, size, content_hash, \
             location, deleted FROM node_version WHERE node_id = ?1 ORDER BY version DESC",
        )?;
        let rows = stmt.query_map(params![node_id.as_slice()], row_to_version)?;
        self.dec_versions(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn find_node_id_in_history(
        &self,
        parent_id: Option<NodeId>,
        name: &str,
    ) -> Result<Option<NodeId>, IndexError> {
        Ok(self
            .conn
            .query_row(
                "SELECT node_id FROM node_version WHERE parent_id IS ?1 AND name = ?2 \
                 ORDER BY version DESC LIMIT 1",
                params![parent_id.map(|p| p.0.to_vec()), self.enc(name)?],
                |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    Ok(NodeId(to_array16(&bytes)))
                },
            )
            .optional()?)
    }

    pub fn versions_beyond_limit(
        &self,
        node_id: NodeId,
        keep: usize,
    ) -> Result<Vec<VersionRow>, IndexError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT node_id, version, created_at, kind, parent_id, name, size, content_hash, \
             location, deleted FROM ( \
                 SELECT nv.*, \
                        ROW_NUMBER() OVER (ORDER BY nv.version DESC) AS rn \
                 FROM node_version nv \
                 WHERE nv.node_id = ?1 AND nv.location = ?2 \
             ) ranked WHERE rn > ?3 \
             ORDER BY version DESC",
        )?;
        let keep_i64 = i64::try_from(keep).unwrap_or(i64::MAX);
        let rows = stmt.query_map(
            params![
                node_id.as_slice(),
                VersionLocation::Archived.to_i64(),
                keep_i64
            ],
            row_to_version,
        )?;
        self.dec_versions(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn versions_beyond_limit_for_root(
        &self,
        root_id: RootId,
        keep: usize,
    ) -> Result<Vec<VersionRow>, IndexError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT node_id, version, created_at, kind, parent_id, name, size, content_hash, \
             location, deleted FROM ( \
                 SELECT nv.*, \
                        ROW_NUMBER() OVER (PARTITION BY nv.node_id ORDER BY nv.version DESC) AS rn \
                 FROM node_version nv \
                 JOIN node n ON n.node_id = nv.node_id \
                 WHERE n.root_id = ?1 AND n.kind = ?2 AND nv.location = ?3 \
             ) ranked WHERE rn > ?4 \
             ORDER BY node_id, version DESC",
        )?;
        let keep_i64 = i64::try_from(keep).unwrap_or(i64::MAX);
        let rows = stmt.query_map(
            params![
                root_id,
                NodeKind::File.to_i64(),
                VersionLocation::Archived.to_i64(),
                keep_i64
            ],
            row_to_version,
        )?;
        self.dec_versions(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn count_versions_beyond_for_root(
        &self,
        root_id: RootId,
        keep: usize,
    ) -> Result<u64, IndexError> {
        let keep_i64 = i64::try_from(keep).unwrap_or(i64::MAX);
        let count: i64 = self
            .conn
            .prepare_cached(
                "SELECT COUNT(*) FROM ( \
                 SELECT ROW_NUMBER() OVER (PARTITION BY nv.node_id ORDER BY nv.version DESC) AS rn \
                 FROM node_version nv \
                 JOIN node n ON n.node_id = nv.node_id \
                 WHERE n.root_id = ?1 AND n.kind = ?2 AND nv.location = ?3 \
             ) ranked WHERE rn > ?4",
            )?
            .query_row(
                params![
                    root_id,
                    NodeKind::File.to_i64(),
                    VersionLocation::Archived.to_i64(),
                    keep_i64
                ],
                |row| row.get(0),
            )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    pub fn root_archived_version_totals(&self, root_id: RootId) -> Result<(u64, u64), IndexError> {
        let (count, bytes): (i64, i64) = self
            .conn
            .prepare_cached(
                "SELECT COUNT(*), COALESCE(SUM(nv.size), 0) FROM node_version nv \
             JOIN node n ON n.node_id = nv.node_id \
             WHERE n.root_id = ?1 AND nv.location = ?2 AND nv.deleted = 0",
            )?
            .query_row(
                params![root_id, VersionLocation::Archived.to_i64()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
        Ok((
            u64::try_from(count).unwrap_or(0),
            u64::try_from(bytes).unwrap_or(0),
        ))
    }

    pub fn delete_version(&self, node_id: NodeId, version: i64) -> Result<(), IndexError> {
        self.conn.execute(
            "DELETE FROM node_version WHERE node_id = ?1 AND version = ?2",
            params![node_id.as_slice(), version],
        )?;
        Ok(())
    }

    pub fn set_version_location(
        &self,
        node_id: NodeId,
        version: i64,
        location: VersionLocation,
    ) -> Result<(), IndexError> {
        self.conn
            .prepare_cached(
                "UPDATE node_version SET location = ?3 WHERE node_id = ?1 AND version = ?2",
            )?
            .execute(params![node_id.as_slice(), version, location.to_i64()])?;
        Ok(())
    }

    pub fn start_run(&self, root_id: RootId, started_at_ms: i64) -> Result<i64, IndexError> {
        self.conn.execute(
            "INSERT INTO run (root_id, started_at) VALUES (?1, ?2)",
            params![root_id, started_at_ms],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn finish_run(
        &self,
        run_id: i64,
        finished_at_ms: i64,
        outcome: RunOutcome,
        files_uploaded: u64,
        files_deleted: u64,
        files_renamed: u64,
        bytes_transferred: u64,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            "UPDATE run SET finished_at = ?2, outcome = ?3, files_uploaded = ?4, \
             files_deleted = ?5, files_renamed = ?6, bytes_transferred = ?7 WHERE run_id = ?1",
            params![
                run_id,
                finished_at_ms,
                outcome.to_i64(),
                u64_to_sql(files_uploaded),
                u64_to_sql(files_deleted),
                u64_to_sql(files_renamed),
                u64_to_sql(bytes_transferred),
            ],
        )?;
        Ok(())
    }

    pub fn set_run_modified(&self, run_id: i64, files_modified: u64) -> Result<(), IndexError> {
        self.conn.execute(
            "UPDATE run SET files_modified = ?2 WHERE run_id = ?1",
            params![run_id, u64_to_sql(files_modified)],
        )?;
        Ok(())
    }

    pub fn mark_unfinished_runs_interrupted(
        &self,
        root_id: RootId,
        except_run_id: i64,
        finished_at_ms: i64,
    ) -> Result<u64, IndexError> {
        let changed = self.conn.execute(
            "UPDATE run SET finished_at = ?3, outcome = ?4 \
             WHERE root_id = ?1 AND run_id != ?2 AND finished_at IS NULL",
            params![
                root_id,
                except_run_id,
                finished_at_ms,
                RunOutcome::Interrupted.to_i64()
            ],
        )?;
        Ok(u64::try_from(changed).unwrap_or(0))
    }

    pub fn get_run(&self, run_id: i64) -> Result<Option<RunRow>, IndexError> {
        Ok(self
            .conn
            .query_row(
                "SELECT run_id, root_id, started_at, finished_at, outcome, files_uploaded, \
                 files_deleted, files_renamed, bytes_transferred, files_modified FROM run WHERE run_id = ?1",
                params![run_id],
                row_to_run,
            )
            .optional()?)
    }

    pub fn list_runs(&self, root_id: RootId, limit: u32) -> Result<Vec<RunRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, root_id, started_at, finished_at, outcome, files_uploaded, \
             files_deleted, files_renamed, bytes_transferred, files_modified FROM run \
             WHERE root_id = ?1 \
             ORDER BY run_id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![root_id, limit], row_to_run)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn sum_bytes_transferred_ok(&self, root_id: RootId) -> Result<u64, IndexError> {
        let bytes: i64 = self.conn.prepare_cached(
            "SELECT COALESCE(SUM(bytes_transferred), 0) FROM run WHERE root_id = ?1 AND outcome = 0",
        )?
        .query_row(params![root_id], |row| row.get(0))?;
        Ok(u64::try_from(bytes).unwrap_or(0))
    }

    pub fn ok_runs_beyond_limit(
        &self,
        root_id: RootId,
        keep: usize,
    ) -> Result<Vec<i64>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id FROM run WHERE root_id = ?1 AND outcome = 0 ORDER BY started_at DESC",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![root_id], |r| r.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids.into_iter().skip(keep).collect())
    }

    pub fn delete_run(&self, run_id: i64) -> Result<(), IndexError> {
        self.conn
            .execute("DELETE FROM run WHERE run_id = ?1", params![run_id])?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn journal_plan(
        &self,
        run_id: i64,
        root_id: RootId,
        node_id: NodeId,
        seq: i64,
        op: JournalOp,
        payload: &str,
        now_ms: i64,
    ) -> Result<i64, IndexError> {
        self.conn.prepare_cached(
            "INSERT INTO journal (run_id, root_id, node_id, seq, op, state, payload, updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        )?.execute(params![
                run_id,
                root_id,
                node_id.as_slice(),
                seq,
                op.to_i64(),
                JournalState::Planned.to_i64(),
                self.enc(payload)?,
                now_ms,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn journal_advance(
        &self,
        entry_id: i64,
        state: JournalState,
        now_ms: i64,
    ) -> Result<(), IndexError> {
        self.conn
            .prepare_cached("UPDATE journal SET state = ?2, updated_at = ?3 WHERE entry_id = ?1")?
            .execute(params![entry_id, state.to_i64(), now_ms])?;
        Ok(())
    }

    pub fn journal_pending(&self, root_id: RootId) -> Result<Vec<JournalEntryRow>, IndexError> {
        const _: () = assert!(JournalState::Committed.to_i64() == 3);
        let mut stmt = self.conn.prepare_cached(
            "SELECT entry_id, run_id, root_id, node_id, seq, op, state, payload, updated_at \
             FROM journal WHERE root_id = ?1 AND state != 3 ORDER BY seq",
        )?;
        let rows = stmt.query_map(params![root_id], row_to_journal_entry)?;
        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|mut entry: JournalEntryRow| {
                entry.payload = self.dec(entry.payload)?;
                Ok(entry)
            })
            .collect()
    }

    pub fn get_manifest_head(
        &self,
        vault_id: &[u8; 32],
    ) -> Result<Option<ManifestHeadRow>, IndexError> {
        self.conn
            .query_row(
                "SELECT vault_id, seq, payload_hash, mac, device_id, updated_at \
                 FROM manifest_head WHERE vault_id = ?1",
                params![vault_id.as_slice()],
                |r| {
                    let vault_id: Vec<u8> = r.get(0)?;
                    let payload_hash: Vec<u8> = r.get(2)?;
                    let device_id: Vec<u8> = r.get(4)?;
                    Ok(ManifestHeadRow {
                        vault_id: to_array32(&vault_id),
                        seq: r.get(1)?,
                        payload_hash: to_array32(&payload_hash),
                        mac: r.get(3)?,
                        device_id: to_array16(&device_id),
                        updated_at: r.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_manifest_head(&self, head: &ManifestHeadRow) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO manifest_head (vault_id, seq, payload_hash, mac, device_id, updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT (vault_id) DO UPDATE SET \
                seq = excluded.seq, payload_hash = excluded.payload_hash, \
                mac = excluded.mac, device_id = excluded.device_id, \
                updated_at = excluded.updated_at",
            params![
                head.vault_id.as_slice(),
                head.seq,
                head.payload_hash.as_slice(),
                head.mac,
                head.device_id.as_slice(),
                head.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn advance_manifest_head(&self, head: &ManifestHeadRow) -> Result<bool, IndexError> {
        let changed = self.conn.execute(
            "INSERT INTO manifest_head (vault_id, seq, payload_hash, mac, device_id, updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT (vault_id) DO UPDATE SET \
                seq = excluded.seq, payload_hash = excluded.payload_hash, \
                mac = excluded.mac, device_id = excluded.device_id, \
                updated_at = excluded.updated_at \
             WHERE excluded.seq > manifest_head.seq",
            params![
                head.vault_id.as_slice(),
                head.seq,
                head.payload_hash.as_slice(),
                head.mac,
                head.device_id.as_slice(),
                head.updated_at,
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn resume_begin(
        &self,
        remote_id: &str,
        object_key: &str,
        part_size: u64,
        source_hash: [u8; 32],
        now_ms: i64,
    ) -> Result<(), IndexError> {
        let existing: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT source_hash FROM remote_upload WHERE remote_id = ?1 AND object_key = ?2",
                params![remote_id, object_key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing_hash) = existing {
            if to_array32(&existing_hash) == source_hash {
                return Ok(());
            }
            self.conn.execute(
                "DELETE FROM remote_upload WHERE remote_id = ?1 AND object_key = ?2",
                params![remote_id, object_key],
            )?;
        }
        self.conn.execute(
            "INSERT INTO remote_upload \
             (remote_id, object_key, part_size, source_hash, created_at, updated_at) \
             VALUES (?1,?2,?3,?4,?5,?5)",
            params![
                remote_id,
                object_key,
                u64_to_sql(part_size),
                source_hash.to_vec(),
                now_ms,
            ],
        )?;
        Ok(())
    }

    pub fn resume_load(
        &self,
        remote_id: &str,
        object_key: &str,
    ) -> Result<Option<RemoteUploadRow>, IndexError> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {REMOTE_UPLOAD_COLUMNS} FROM remote_upload \
                     WHERE remote_id = ?1 AND object_key = ?2"
                ),
                params![remote_id, object_key],
                row_to_remote_upload,
            )
            .optional()?)
    }

    pub fn resume_set_upload_id(
        &self,
        remote_id: &str,
        object_key: &str,
        upload_id: &str,
        now_ms: i64,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            "UPDATE remote_upload SET upload_id = ?3, updated_at = ?4 \
             WHERE remote_id = ?1 AND object_key = ?2",
            params![remote_id, object_key, upload_id, now_ms],
        )?;
        Ok(())
    }

    pub fn resume_set_prologue(
        &self,
        remote_id: &str,
        object_key: &str,
        prologue: &[u8],
        now_ms: i64,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            "UPDATE remote_upload SET prologue = ?3, updated_at = ?4 \
             WHERE remote_id = ?1 AND object_key = ?2",
            params![remote_id, object_key, prologue, now_ms],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resume_record_part(
        &self,
        upload_row_id: i64,
        part_number: u32,
        etag: &str,
        crc32c: [u8; 4],
        len: u64,
        now_ms: i64,
    ) -> Result<(), IndexError> {
        self.conn.execute(
            "INSERT INTO remote_upload_part \
             (upload_row_id, part_number, etag, crc32c, len, uploaded_at) \
             VALUES (?1,?2,?3,?4,?5,?6) \
             ON CONFLICT (upload_row_id, part_number) \
             DO UPDATE SET etag = excluded.etag, crc32c = excluded.crc32c, \
                            len = excluded.len, uploaded_at = excluded.uploaded_at",
            params![
                upload_row_id,
                part_number,
                etag,
                crc32c.to_vec(),
                u64_to_sql(len),
                now_ms,
            ],
        )?;
        self.conn.execute(
            "UPDATE remote_upload SET updated_at = ?2 WHERE upload_row_id = ?1",
            params![upload_row_id, now_ms],
        )?;
        Ok(())
    }

    pub fn resume_parts(&self, upload_row_id: i64) -> Result<Vec<RemoteUploadPartRow>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT part_number, etag, crc32c, len, uploaded_at FROM remote_upload_part \
             WHERE upload_row_id = ?1 ORDER BY part_number",
        )?;
        let rows = stmt.query_map(params![upload_row_id], row_to_remote_upload_part)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn resume_finish(&self, remote_id: &str, object_key: &str) -> Result<(), IndexError> {
        self.conn.execute(
            "DELETE FROM remote_upload WHERE remote_id = ?1 AND object_key = ?2",
            params![remote_id, object_key],
        )?;
        Ok(())
    }

    pub fn resume_abort(&self, remote_id: &str, object_key: &str) -> Result<(), IndexError> {
        self.resume_finish(remote_id, object_key)
    }

    pub fn resume_list_stale(
        &self,
        remote_id: &str,
        older_than_ms: i64,
    ) -> Result<Vec<(String, Option<String>)>, IndexError> {
        let mut stmt = self.conn.prepare(
            "SELECT object_key, upload_id FROM remote_upload \
             WHERE remote_id = ?1 AND updated_at < ?2",
        )?;
        let rows = stmt.query_map(params![remote_id, older_than_ms], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn row_to_version(row: &Row<'_>) -> rusqlite::Result<VersionRow> {
    let node_id: Vec<u8> = row.get(0)?;
    let parent_id: Option<Vec<u8>> = row.get(4)?;
    let content_hash: Option<Vec<u8>> = row.get(7)?;
    #[allow(clippy::cast_sign_loss)]
    let size = row.get::<_, i64>(6)? as u64;
    Ok(VersionRow {
        node_id: NodeId(to_array16(&node_id)),
        version: row.get(1)?,
        created_at: row.get(2)?,
        kind: NodeKind::from_i64(row.get(3)?),
        parent_id: parent_id.map(|b| NodeId(to_array16(&b))),
        name: row.get(5)?,
        size,
        content_hash: content_hash.map(|b| to_array32(&b)),
        location: VersionLocation::from_i64(row.get(8)?),
        deleted: row.get::<_, i64>(9)? != 0,
    })
}

fn row_to_journal_entry(row: &Row<'_>) -> rusqlite::Result<JournalEntryRow> {
    let node_id: Vec<u8> = row.get(3)?;
    Ok(JournalEntryRow {
        entry_id: row.get(0)?,
        run_id: row.get(1)?,
        root_id: row.get(2)?,
        node_id: NodeId(to_array16(&node_id)),
        seq: row.get(4)?,
        op: JournalOp::from_i64(row.get(5)?),
        state: JournalState::from_i64(row.get(6)?),
        payload: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn row_to_run(row: &Row<'_>) -> rusqlite::Result<RunRow> {
    let outcome: Option<i64> = row.get(4)?;
    #[allow(clippy::cast_sign_loss)]
    Ok(RunRow {
        run_id: row.get(0)?,
        root_id: row.get(1)?,
        started_at: row.get(2)?,
        finished_at: row.get(3)?,
        outcome: outcome.map(RunOutcome::from_i64),
        files_uploaded: row.get::<_, i64>(5)? as u64,
        files_deleted: row.get::<_, i64>(6)? as u64,
        files_renamed: row.get::<_, i64>(7)? as u64,
        bytes_transferred: row.get::<_, i64>(8)? as u64,
        files_modified: row.get::<_, Option<i64>>(9)?.map(|n| n as u64),
    })
}

fn migrate(conn: &Connection) -> Result<(), IndexError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let known = schema::current_version();
    if found > known {
        return Err(IndexError::SchemaTooNew { found, known });
    }
    for migration in schema::MIGRATIONS {
        if migration.version <= found {
            continue;
        }
        let tx: Transaction<'_> = conn.unchecked_transaction()?;
        tx.execute_batch(migration.sql)?;
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(windows)]
fn restrict_permissions(path: &Path) {
    use winapi::um::winnt::{FILE_ALL_ACCESS, PSID};
    use windows_acl::acl::ACL;
    use windows_acl::helper::{current_user, name_to_sid, sid_to_string};

    let Some(path_str) = path.to_str() else {
        return;
    };
    let Some(user) = current_user() else {
        return;
    };
    let Ok(user_sid) = name_to_sid(&user, None) else {
        return;
    };
    let Ok(user_sid_string) = sid_to_string(user_sid.as_ptr() as PSID) else {
        return;
    };
    let Ok(mut acl) = ACL::from_file_path(path_str, false) else {
        return;
    };
    let to_remove: Vec<Vec<u16>> = acl
        .all()
        .map(|entries| {
            entries
                .into_iter()
                .filter(|entry| entry.string_sid != user_sid_string)
                .filter_map(|entry| entry.sid)
                .collect()
        })
        .unwrap_or_default();
    if acl
        .allow(user_sid.as_ptr() as PSID, false, FILE_ALL_ACCESS)
        .is_err()
    {
        return;
    }
    for sid in &to_remove {
        let _ = acl.remove(sid.as_ptr() as PSID, None, None);
    }
}

#[cfg(not(any(unix, windows)))]
fn restrict_permissions(_path: &Path) {}
