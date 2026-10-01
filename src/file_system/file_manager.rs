use crate::file_system::file_meta::FileMeta;
use std::{env, fs, fs::File, path::PathBuf};

use diesel::{Connection, SqliteConnection};

const META_PATH: &str = "meta.db";
const WORKSPACE_DIR: &str = ".netdisk";

fn workspace_dir() -> PathBuf {
    let home = env::var("HOME").expect("HOME environment variable is not set");
    PathBuf::from(home).join(WORKSPACE_DIR)
}

fn establish_connection() -> SqliteConnection {
    let dir = workspace_dir();
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("Error creating {}: {e}", dir.display()));
    let path = dir.join(META_PATH);
    SqliteConnection::establish(path.to_str().unwrap())
        .unwrap_or_else(|_| panic!("Error connecting to {}", path.display()))
}

pub struct FileManager {
    db: SqliteConnection,
}


impl FileManager {
    pub fn new() -> Self {
        let db = establish_connection();
        FileManager { db }
    }

    pub fn new_file(&self, file_meta: FileMeta) -> File {
        File::create(&file_meta.file_hash).unwrap()
        // Insert file_meta into the database
        
        
        
    }
}
