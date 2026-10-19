CREATE TABLE remote_upload (
    upload_row_id INTEGER PRIMARY KEY AUTOINCREMENT,
    remote_id     TEXT NOT NULL,
    object_key    TEXT NOT NULL,
    upload_id     TEXT,
    part_size     INTEGER NOT NULL,
    prologue      BLOB,
    source_hash   BLOB NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE (remote_id, object_key)
);

CREATE TABLE remote_upload_part (
    upload_row_id INTEGER NOT NULL REFERENCES remote_upload (upload_row_id) ON DELETE CASCADE,
    part_number   INTEGER NOT NULL,
    etag          TEXT NOT NULL,
    crc32c        BLOB NOT NULL,
    len           INTEGER NOT NULL,
    uploaded_at   INTEGER NOT NULL,
    PRIMARY KEY (upload_row_id, part_number)
);

CREATE INDEX remote_upload_stale ON remote_upload (remote_id, updated_at);
