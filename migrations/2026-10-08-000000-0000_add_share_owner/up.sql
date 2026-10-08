ALTER TABLE share_table ADD COLUMN owner_id TEXT;
-- Old shares did not record the creator. Infer ownership only for a single source owner.
UPDATE share_table SET owner_id = (
    SELECT MIN(file_meta.file_owner) FROM share_files
    JOIN file_meta ON file_meta.file_id = share_files.file_id
    WHERE share_files.dic_id = share_table.dic_id
    HAVING COUNT(DISTINCT file_meta.file_owner) = 1
);
CREATE INDEX share_table_owner_id_idx ON share_table(owner_id);
