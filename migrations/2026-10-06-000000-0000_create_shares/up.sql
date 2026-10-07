CREATE TABLE share_table (
    share_code TEXT PRIMARY KEY NOT NULL
        CHECK (length(share_code) = 6 AND share_code NOT GLOB '*[^0-9]*'),
    dic_id TEXT NOT NULL UNIQUE,
    created_at TIMESTAMP NOT NULL,
    expired_at TIMESTAMP,
    CHECK (expired_at IS NULL OR expired_at > created_at)
);

-- dic_id identifies a collection; sharing never reparents the source files.
CREATE TABLE share_files (
    dic_id TEXT NOT NULL REFERENCES share_table(dic_id) ON DELETE CASCADE,
    file_id TEXT NOT NULL REFERENCES file_meta(file_id) ON DELETE CASCADE,
    PRIMARY KEY (dic_id, file_id)
);

CREATE INDEX share_files_file_id_idx ON share_files(file_id);
CREATE INDEX share_table_expired_at_idx ON share_table(expired_at);
