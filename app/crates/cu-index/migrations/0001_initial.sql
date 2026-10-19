CREATE TABLE root (
    root_id      INTEGER PRIMARY KEY AUTOINCREMENT,
    local_path   TEXT NOT NULL,
    vault_path   TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    UNIQUE (local_path, vault_path)
);

CREATE TABLE node (
    node_id       BLOB PRIMARY KEY,
    root_id       INTEGER NOT NULL REFERENCES root (root_id) ON DELETE CASCADE,
    parent_id     BLOB REFERENCES node (node_id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          INTEGER NOT NULL,
    size          INTEGER NOT NULL DEFAULT 0,
    mtime_ns      INTEGER NOT NULL DEFAULT 0,
    content_hash  BLOB,
    version       INTEGER NOT NULL DEFAULT 0,
    scan_dev      INTEGER,
    scan_ino      INTEGER,
    scan_size     INTEGER,
    scan_mtime_ns INTEGER,
    UNIQUE (root_id, parent_id, name)
);
CREATE INDEX node_root_parent ON node (root_id, parent_id);

CREATE TABLE node_version (
    node_id      BLOB NOT NULL,
    version      INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    kind         INTEGER NOT NULL,
    parent_id    BLOB,
    name         TEXT NOT NULL,
    size         INTEGER NOT NULL DEFAULT 0,
    content_hash BLOB,
    location     INTEGER NOT NULL,
    deleted      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (node_id, version)
);

CREATE TABLE run (
    run_id             INTEGER PRIMARY KEY AUTOINCREMENT,
    root_id            INTEGER NOT NULL REFERENCES root (root_id) ON DELETE CASCADE,
    started_at         INTEGER NOT NULL,
    finished_at        INTEGER,
    outcome            INTEGER,
    files_uploaded     INTEGER NOT NULL DEFAULT 0,
    files_deleted      INTEGER NOT NULL DEFAULT 0,
    files_renamed      INTEGER NOT NULL DEFAULT 0,
    bytes_transferred  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE journal (
    entry_id   INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id     INTEGER NOT NULL REFERENCES run (run_id) ON DELETE CASCADE,
    root_id    INTEGER NOT NULL REFERENCES root (root_id) ON DELETE CASCADE,
    node_id    BLOB NOT NULL,
    seq        INTEGER NOT NULL,
    op         INTEGER NOT NULL,
    state      INTEGER NOT NULL,
    payload    TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX journal_run_seq ON journal (run_id, seq);
CREATE INDEX journal_pending ON journal (root_id, state) WHERE state != 3;

CREATE TABLE manifest_entry (
    root_id      INTEGER NOT NULL REFERENCES root (root_id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL,
    node_id      BLOB NOT NULL,
    version      INTEGER NOT NULL,
    content_hash BLOB,
    mac          BLOB NOT NULL,
    PRIMARY KEY (root_id, seq)
);
