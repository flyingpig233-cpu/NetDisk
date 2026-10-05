use crate::db::DbConn;
use crate::db_types::UuidSql;
use crate::file_system::file_meta::FileMeta;
use crate::schema::file_meta;
use std::{fs, path::PathBuf};

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, RunQueryDsl, SqliteConnection};
use uuid::Uuid;

const STORE_DIR: &str = ".netdisk_store/";

pub const ROOT_ID: Uuid = Uuid::nil();

pub fn get_store_dir() -> PathBuf {
    let mut store_dir = dirs::home_dir().expect("Could not find home directory");
    store_dir.push(STORE_DIR);
    if !store_dir.exists() {
        fs::create_dir_all(&store_dir).expect("Could not create store directory");
    }
    store_dir
}

#[derive(Debug)]
pub enum FileError {
    NotFound,
    InvalidMove,
    Database(diesel::result::Error),
    Io(std::io::Error),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::NotFound => write!(f, "file not found"),
            FileError::InvalidMove => {
                write!(f, "cannot move a folder into itself or its descendants")
            }
            FileError::Database(e) => write!(f, "database error: {e}"),
            FileError::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for FileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FileError::NotFound => None,
            FileError::InvalidMove => None,
            FileError::Database(e) => Some(e),
            FileError::Io(e) => Some(e),
        }
    }
}

impl From<diesel::result::Error> for FileError {
    fn from(value: diesel::result::Error) -> Self {
        FileError::Database(value)
    }
}

impl From<std::io::Error> for FileError {
    fn from(value: std::io::Error) -> Self {
        FileError::Io(value)
    }
}

pub fn ensure_root(conn: &mut SqliteConnection) -> Result<(), FileError> {
    use crate::schema::file_meta::dsl::*;

    let exists = file_meta
        .filter(file_id.eq(UuidSql::from(ROOT_ID)))
        .first::<FileMeta>(conn)
        .optional()?
        .is_some();

    if exists {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp() as u32;
    let root = FileMeta {
        file_id: ROOT_ID,
        file_name: "/".to_string(),
        file_size: 0,
        file_hash: String::new(),
        file_owner: ROOT_ID,
        file_created_at: now,
        file_updated_at: now,
        parent_id: ROOT_ID,
        is_directory: true,
    };

    diesel::insert_into(file_meta).values(root).execute(conn)?;

    Ok(())
}

#[derive(Debug, Clone)]
pub struct ZipFile {
    pub zip_path: String,
    pub real_path: PathBuf,
}

pub struct FileManager {
    db: DbConn,
}

impl FileManager {
    pub fn new(db: DbConn) -> Self {
        FileManager { db }
    }

    pub fn new_file(&mut self, meta: FileMeta) -> Result<(), FileError> {
        diesel::insert_into(file_meta::table)
            .values(meta)
            .execute(&mut self.db)?;

        Ok(())
    }

    pub fn delete_file(&mut self, file_uid: Uuid) -> Result<(), FileError> {
        use crate::schema::file_meta::dsl::*;

        let target_file = file_meta
            .filter(file_id.eq(UuidSql::from(file_uid)))
            .first::<FileMeta>(&mut self.db)
            .optional()?
            .ok_or(FileError::NotFound)?;

        if target_file.is_directory {
            let child_files = file_meta
                .filter(parent_id.eq(UuidSql::from(file_uid)))
                .load::<FileMeta>(&mut self.db)?;

            for child in child_files {
                // Recursively delete child files and directories
                self.delete_file(child.file_id)?;
            }
        }

        let file_path = get_store_dir().join(&target_file.file_hash);
        if !file_path.exists() {
            return Err(FileError::NotFound);
        }
        diesel::delete(file_meta.filter(file_id.eq(UuidSql::from(file_uid))))
            .execute(&mut self.db)?;

        if file_meta
            .filter(file_hash.eq(&target_file.file_hash))
            .count()
            .get_result::<i64>(&mut self.db)?
            > 0
        {
            // There are still other files with the same hash, so we don't delete the physical file
            return Ok(());
        }
        fs::remove_file(&file_path)?;

        Ok(())
    }

    pub fn rename_file(&mut self, file_uid: Uuid, new_name: String) -> Result<(), FileError> {
        use crate::schema::file_meta::dsl::*;

        let affected = diesel::update(file_meta.filter(file_id.eq(UuidSql::from(file_uid))))
            .set(file_name.eq(new_name))
            .execute(&mut self.db)?;

        if affected == 0 {
            return Err(FileError::NotFound);
        }
        Ok(())
    }

    pub fn move_file(
        &mut self,
        file_uid: Uuid,
        new_parent_uid: Option<Uuid>,
    ) -> Result<(), FileError> {
        use crate::schema::file_meta::dsl::*;

        let new_parent_id = match new_parent_uid {
            Some(uid) => uid,
            None => ROOT_ID,
        };

        // Reject moving an item into itself or into one of its own descendants,
        // which would detach the subtree from the root and orphan it.
        if file_uid == new_parent_id {
            return Err(FileError::InvalidMove);
        }
        let mut cursor = new_parent_id;
        let mut hops = 0u32;
        loop {
            if cursor == ROOT_ID {
                break;
            }
            if cursor == file_uid {
                return Err(FileError::InvalidMove);
            }
            hops += 1;
            if hops > 100_000 {
                return Err(FileError::InvalidMove);
            }
            let parent = file_meta
                .filter(file_id.eq(UuidSql::from(cursor)))
                .select(parent_id)
                .first::<UuidSql>(&mut self.db)
                .optional()?;
            match parent {
                Some(value) => cursor = value.0,
                None => break,
            }
        }

        let affected = diesel::update(file_meta.filter(file_id.eq(UuidSql::from(file_uid))))
            .set(parent_id.eq(UuidSql::from(new_parent_id)))
            .execute(&mut self.db)?;

        if affected == 0 {
            return Err(FileError::NotFound);
        }
        Ok(())
    }

    pub fn get_file_meta(&mut self, file_uid: Uuid) -> Result<Option<FileMeta>, FileError> {
        use crate::schema::file_meta::dsl::*;

        Ok(file_meta
            .filter(file_id.eq(UuidSql::from(file_uid)))
            .first::<FileMeta>(&mut self.db)
            .optional()?)
    }

    pub fn get_file_list(
        &mut self,
        user_id: Uuid,
        parent_uid: Uuid,
    ) -> Result<Vec<FileMeta>, FileError> {
        use crate::schema::file_meta::dsl::*;

        Ok(file_meta
            .filter(file_owner.eq(UuidSql::from(user_id)))
            .filter(parent_id.eq(UuidSql::from(parent_uid)))
            .load::<FileMeta>(&mut self.db)?)
    }

    pub fn list_dict_recursive(
        &mut self,
        dir_uid: Uuid,
        path_prefix: String,
    ) -> Result<Vec<ZipFile>, FileError> {
        use crate::schema::file_meta::dsl::*;

        let mut result = Vec::new();
        let children = file_meta
            .filter(parent_id.eq(UuidSql::from(dir_uid)))
            .load::<FileMeta>(&mut self.db)?;

        for child in children {
            if child.is_directory {
                let new_prefix = format!("{}{}/", path_prefix, child.file_name);
                let mut sub_children = self.list_dict_recursive(child.file_id, new_prefix)?;
                result.append(&mut sub_children);
            } else {
                result.push(ZipFile {
                    zip_path: format!("{}{}", path_prefix, child.file_name),
                    real_path: get_store_dir().join(&child.file_hash),
                });
            }
        }

        Ok(result)
    }
}
