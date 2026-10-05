use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbConn;
use crate::db_types::UuidSql;

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
        })
    }

    pub fn verify_password(&self, password: &str) -> bool {
        verify_password(password, &self.password_hash)
    }
}

pub struct UserManager;

impl UserManager {
    pub fn create_user(
        conn: &mut DbConn,
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
        conn: &mut DbConn,
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
        conn: &mut DbConn,
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
