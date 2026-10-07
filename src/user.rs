use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db_types::UuidSql;
use diesel::SqliteConnection;

#[derive(Debug, Clone, Serialize, Deserialize, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::schema::users)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct User {
    #[diesel(serialize_as = UuidSql, deserialize_as = UuidSql)]
    pub user_id: Uuid,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    #[diesel(serialize_as = i64, deserialize_as = i64)]
    pub created_at: u32,
    #[diesel(serialize_as = i64, deserialize_as = i64)]
    pub updated_at: u32,
    pub is_admin: bool,
    #[serde(skip_serializing)]
    pub token_version: i64,
}

impl User {
    pub fn new(username: String, password: &str) -> Result<Self, argon2::password_hash::Error> {
        let now = chrono::Utc::now().timestamp() as u32;

        Ok(User {
            user_id: Uuid::new_v4(),
            username,
            password_hash: hash_password(password)?,
            created_at: now,
            updated_at: now,
            is_admin: false,
            token_version: 0,
        })
    }

    pub fn verify_password(&self, password: &str) -> bool {
        verify_password(password, &self.password_hash)
    }
}

pub struct UserManager;

impl UserManager {
    pub fn create_user(
        conn: &mut SqliteConnection,
        name: String,
        password: &str,
    ) -> Result<User, diesel::result::Error> {
        use crate::schema::users::dsl::*;

        let new_user =
            User::new(name, password).map_err(|_| diesel::result::Error::RollbackTransaction)?;

        diesel::insert_into(users)
            .values(new_user.clone())
            .execute(conn)?;

        Ok(new_user)
    }

    pub fn get_user_by_username(
        conn: &mut SqliteConnection,
        user_name: &str,
    ) -> Result<Option<User>, diesel::result::Error> {
        use crate::schema::users::dsl::*;

        let user = users
            .filter(username.eq(user_name))
            .first::<User>(conn)
            .optional()?;

        Ok(user)
    }

    pub fn get_user_by_id(
        conn: &mut SqliteConnection,
        user_id_val: Uuid,
    ) -> Result<Option<User>, diesel::result::Error> {
        use crate::schema::users::dsl::*;

        let user = users
            .filter(user_id.eq(UuidSql::from(user_id_val)))
            .first::<User>(conn)
            .optional()?;

        Ok(user)
    }
}

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let hash = Argon2::default()
        .hash_password(password.as_bytes())?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, password_hash: &str) -> bool {
    Argon2::default()
        .verify_password(password.as_bytes(), password_hash)
        .is_ok()
}

/// Bootstrap once; never overwrite an existing account or reset its password on restart.
pub fn ensure_admin(conn: &mut SqliteConnection, password: &str) -> Result<Option<User>, String> {
    use crate::schema::users::dsl::*;
    if password.len() < 8 {
        return Err("ADMIN_PASSWORD must be at least 8 bytes".into());
    }
    conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
        if users
            .filter(is_admin.eq(true))
            .count()
            .get_result::<i64>(conn)?
            > 0
        {
            return Ok(None);
        }
        if users
            .filter(username.eq("admin"))
            .first::<User>(conn)
            .optional()?
            .is_some()
        {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        let mut admin = User::new("admin".into(), password)
            .map_err(|_| diesel::result::Error::RollbackTransaction)?;
        admin.is_admin = true;
        diesel::insert_into(users)
            .values(admin.clone())
            .execute(conn)?;
        Ok(Some(admin))
    })
    .map_err(|error| {
        format!(
            "Unable to initialize admin (an existing admin username will not be promoted): {error}"
        )
    })
}
