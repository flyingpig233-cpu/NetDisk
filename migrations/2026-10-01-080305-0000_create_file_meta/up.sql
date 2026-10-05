CREATE TABLE file_meta (
    file_id TEXT PRIMARY KEY NOT NULL,
    file_name TEXT NOT NULL,
    file_size BIGINT NOT NULL,
    file_hash TEXT NOT NULL,
    file_owner TEXT NOT NULL,
    file_created_at BIGINT NOT NULL,
    file_updated_at BIGINT NOT NULL
);
