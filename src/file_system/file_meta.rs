use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db_types::{U64Sql, UuidSql};

#[derive(Debug, Clone, Serialize, Deserialize, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::file_meta)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct FileMeta {
    #[diesel(serialize_as = UuidSql, deserialize_as = UuidSql)]
    pub file_id: Uuid,

    // real name
    pub file_name: String,
    #[diesel(serialize_as = U64Sql, deserialize_as = U64Sql)]
    pub file_size: u64,
    pub file_hash: String,
    #[diesel(serialize_as = UuidSql, deserialize_as = UuidSql)]
    pub file_owner: Uuid,
    #[diesel(serialize_as = i64, deserialize_as = i64)]
    pub file_created_at: u32,
    #[diesel(serialize_as = i64, deserialize_as = i64)]
    pub file_updated_at: u32,

    #[diesel(serialize_as = UuidSql, deserialize_as = UuidSql)]
    pub parent_id: Uuid,

    pub is_directory: bool,
}
