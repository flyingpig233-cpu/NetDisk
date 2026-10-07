use chrono::{NaiveDateTime, Utc};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db_types::UuidSql;
use crate::file_system::file_meta::FileMeta;
use crate::schema::{file_meta, share_files, share_table};

#[derive(Debug, Clone, Serialize, Deserialize, Queryable, Selectable, Insertable)]
#[diesel(table_name = share_table)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct ShareTable {
    pub share_code: String,
    // A collection of source files, not a physical directory in file_meta.
    #[diesel(serialize_as = UuidSql, deserialize_as = UuidSql)]
    pub dic_id: Uuid,
    pub created_at: NaiveDateTime,
    pub expired_at: Option<NaiveDateTime>,
}

#[derive(Insertable)]
#[diesel(table_name = share_files)]
struct ShareFile {
    dic_id: UuidSql,
    file_id: UuidSql,
}

#[derive(Debug)]
pub enum ShareError {
    EmptyFiles,
    InvalidExpiration,
    NotFound,
    Forbidden,
    CodeExhausted,
    Database(diesel::result::Error),
}

impl From<diesel::result::Error> for ShareError {
    fn from(value: diesel::result::Error) -> Self {
        Self::Database(value)
    }
}

pub struct ShareManager;

impl ShareManager {
    pub fn create_share(
        conn: &mut SqliteConnection,
        owner: Uuid,
        mut file_ids: Vec<Uuid>,
        expiration: Option<i64>,
    ) -> Result<ShareTable, ShareError> {
        if file_ids.is_empty() {
            return Err(ShareError::EmptyFiles);
        }
        file_ids.sort_unstable();
        file_ids.dedup();
        let now = Utc::now().naive_utc();
        let expiration = expiration
            .map(|timestamp| {
                chrono::DateTime::from_timestamp(timestamp, 0)
                    .map(|date| date.naive_utc())
                    .filter(|date| *date > now)
                    .ok_or(ShareError::InvalidExpiration)
            })
            .transpose()?;

        conn.immediate_transaction(|conn| {
            for id in &file_ids {
                let file = file_meta::table
                    .find(UuidSql::from(*id))
                    .first::<FileMeta>(conn)
                    .optional()?
                    .ok_or(ShareError::NotFound)?;
                if file.file_owner != owner || *id == Uuid::nil() {
                    return Err(ShareError::Forbidden);
                }
            }
            // Expired codes can be reused. Foreign keys remove their memberships.
            Self::delete_expired(conn, now)?;
            for _ in 0..32 {
                let share = ShareTable {
                    share_code: format!("{:06}", Uuid::new_v4().as_u128() % 1_000_000),
                    dic_id: Uuid::new_v4(),
                    created_at: now,
                    expired_at: expiration,
                };
                let inserted = diesel::insert_into(share_table::table)
                    .values(share.clone())
                    .on_conflict(share_table::share_code)
                    .do_nothing()
                    .execute(conn)?;
                if inserted == 0 {
                    continue;
                }
                for id in &file_ids {
                    diesel::insert_into(share_files::table)
                        .values(ShareFile {
                            dic_id: share.dic_id.into(),
                            file_id: (*id).into(),
                        })
                        .execute(conn)?;
                }
                return Ok(share);
            }
            Err(ShareError::CodeExhausted)
        })
    }

    pub fn get_share(conn: &mut SqliteConnection, code: &str) -> QueryResult<Option<ShareTable>> {
        let now = Utc::now().naive_utc();
        share_table::table
            .find(code)
            .filter(
                share_table::expired_at
                    .is_null()
                    .or(share_table::expired_at.gt(now)),
            )
            .first(conn)
            .optional()
    }

    pub fn get_files(conn: &mut SqliteConnection, code: &str) -> QueryResult<Vec<FileMeta>> {
        let Some(share) = Self::get_share(conn, code)? else {
            return Err(diesel::result::Error::NotFound);
        };
        share_files::table
            .inner_join(file_meta::table)
            .filter(share_files::dic_id.eq(UuidSql::from(share.dic_id)))
            .select(FileMeta::as_select())
            .load(conn)
    }

    pub fn delete_share(conn: &mut SqliteConnection, code: &str) -> QueryResult<usize> {
        diesel::delete(share_table::table.find(code)).execute(conn)
    }

    pub fn delete_expired(conn: &mut SqliteConnection, now: NaiveDateTime) -> QueryResult<usize> {
        diesel::delete(share_table::table.filter(share_table::expired_at.le(now))).execute(conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::connection::SimpleConnection;

    fn database() -> SqliteConnection {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        conn.batch_execute("PRAGMA foreign_keys = ON;").unwrap();
        for migration in [
            include_str!("../migrations/2026-10-01-080305-0000_create_file_meta/up.sql"),
            include_str!("../migrations/2026-10-01-121330-0000_add_parent_and_is_dir/up.sql"),
            include_str!("../migrations/2026-10-01-122204-0000_add_is_link_and_target/up.sql"),
            include_str!("../migrations/2026-10-01-135407-0000_create_users/up.sql"),
            include_str!("../migrations/2026-10-04-000000-0000_remove_link_fields/up.sql"),
            include_str!("../migrations/2026-10-06-000000-0000_create_shares/up.sql"),
        ] {
            conn.batch_execute(migration).unwrap();
        }
        conn
    }

    fn file(conn: &mut SqliteConnection, owner: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        diesel::insert_into(file_meta::table)
            .values(FileMeta {
                file_id: id,
                file_name: "test.txt".into(),
                file_size: 0,
                file_hash: "test-hash".into(),
                file_owner: owner,
                file_created_at: 0,
                file_updated_at: 0,
                parent_id: Uuid::nil(),
                is_directory: false,
            })
            .execute(conn)
            .unwrap();
        id
    }

    #[test]
    fn persists_multiple_files_deduplicates_and_cascades() {
        let mut conn = database();
        let owner = Uuid::new_v4();
        let first = file(&mut conn, owner);
        let second = file(&mut conn, owner);
        let share =
            ShareManager::create_share(&mut conn, owner, vec![first, second, first], None).unwrap();
        assert_eq!(share.share_code.len(), 6);
        assert!(share.share_code.bytes().all(|b| b.is_ascii_digit()));
        assert!(
            ShareManager::get_share(&mut conn, &share.share_code)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            ShareManager::get_files(&mut conn, &share.share_code)
                .unwrap()
                .len(),
            2
        );
        let source = file_meta::table
            .find(UuidSql::from(first))
            .first::<FileMeta>(&mut conn)
            .unwrap();
        assert_eq!(source.parent_id, Uuid::nil());
        diesel::delete(file_meta::table.find(UuidSql::from(first)))
            .execute(&mut conn)
            .unwrap();
        assert_eq!(
            ShareManager::get_files(&mut conn, &share.share_code)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            ShareManager::delete_share(&mut conn, &share.share_code).unwrap(),
            1
        );
        assert_eq!(
            share_files::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            0
        );
        assert!(
            file_meta::table
                .find(UuidSql::from(second))
                .first::<FileMeta>(&mut conn)
                .is_ok()
        );
    }

    #[test]
    fn rejects_invalid_requests_without_partial_records() {
        let mut conn = database();
        let owner = Uuid::new_v4();
        let own = file(&mut conn, owner);
        let foreign = file(&mut conn, Uuid::new_v4());
        assert!(matches!(
            ShareManager::create_share(&mut conn, owner, vec![], None),
            Err(ShareError::EmptyFiles)
        ));
        assert!(matches!(
            ShareManager::create_share(&mut conn, owner, vec![own, foreign], None),
            Err(ShareError::Forbidden)
        ));
        assert!(matches!(
            ShareManager::create_share(&mut conn, owner, vec![own, Uuid::new_v4()], None),
            Err(ShareError::NotFound)
        ));
        for expiration in [0, Utc::now().timestamp(), i64::MAX] {
            assert!(matches!(
                ShareManager::create_share(&mut conn, owner, vec![own], Some(expiration)),
                Err(ShareError::InvalidExpiration)
            ));
        }
        assert_eq!(
            share_table::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            0
        );
        assert_eq!(
            share_files::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            0
        );
    }

    #[test]
    fn expiration_and_cleanup_need_no_timer() {
        let mut conn = database();
        let owner = Uuid::new_v4();
        let id = file(&mut conn, owner);
        let share = ShareManager::create_share(
            &mut conn,
            owner,
            vec![id],
            Some(Utc::now().timestamp() + 3600),
        )
        .unwrap();
        assert!(
            ShareManager::get_share(&mut conn, &share.share_code)
                .unwrap()
                .is_some()
        );
        diesel::update(share_table::table.find(&share.share_code))
            .set((
                share_table::created_at.eq(Utc::now().naive_utc() - chrono::Duration::hours(2)),
                share_table::expired_at.eq(Utc::now().naive_utc() - chrono::Duration::hours(1)),
            ))
            .execute(&mut conn)
            .unwrap();
        assert!(
            ShareManager::get_share(&mut conn, &share.share_code)
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            ShareManager::get_files(&mut conn, &share.share_code),
            Err(diesel::result::Error::NotFound)
        ));
        assert_eq!(
            ShareManager::delete_expired(&mut conn, Utc::now().naive_utc()).unwrap(),
            1
        );
        assert_eq!(
            share_files::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            0
        );
    }

    #[test]
    fn membership_failure_rolls_back_share_record() {
        let mut conn = database();
        let owner = Uuid::new_v4();
        let id = file(&mut conn, owner);
        conn.batch_execute(
            "CREATE TRIGGER reject_membership BEFORE INSERT ON share_files
             BEGIN SELECT RAISE(ABORT, 'simulated membership failure'); END;",
        )
        .unwrap();
        assert!(matches!(
            ShareManager::create_share(&mut conn, owner, vec![id], None),
            Err(ShareError::Database(_))
        ));
        assert_eq!(
            share_table::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            0
        );
        assert_eq!(
            share_files::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            0
        );
    }

    #[test]
    fn migration_rolls_back_and_reapplies() {
        let mut conn = database();
        conn.batch_execute(include_str!(
            "../migrations/2026-10-06-000000-0000_create_shares/down.sql"
        ))
        .unwrap();
        conn.batch_execute(include_str!(
            "../migrations/2026-10-06-000000-0000_create_shares/up.sql"
        ))
        .unwrap();
        assert_eq!(
            share_table::table
                .count()
                .get_result::<i64>(&mut conn)
                .unwrap(),
            0
        );
    }
}
