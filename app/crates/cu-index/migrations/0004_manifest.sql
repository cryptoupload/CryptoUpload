CREATE TABLE manifest_head (
    vault_id     BLOB PRIMARY KEY,
    seq          INTEGER NOT NULL,
    payload_hash BLOB NOT NULL,
    mac          BLOB NOT NULL,
    device_id    BLOB NOT NULL,
    updated_at   INTEGER NOT NULL
);
