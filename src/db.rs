use diesel::r2d2::{ConnectionManager, CustomizeConnection, Pool, PooledConnection};
use diesel::SqliteConnection;
use diesel::connection::SimpleConnection;
use std::time::Duration;

pub type DbPool = Pool<ConnectionManager<SqliteConnection>>;
pub type DbConn = PooledConnection<ConnectionManager<SqliteConnection>>;

#[derive(Debug)]
pub struct ConnectionOptions {
    pub enable_wal: bool,
    pub busy_timeout: Option<Duration>,
}

impl CustomizeConnection<SqliteConnection, diesel::r2d2::Error> for ConnectionOptions {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), diesel::r2d2::Error> {
        if let Some(d) = self.busy_timeout {
            conn.batch_execute(&format!("PRAGMA busy_timeout = {};", d.as_millis()))
                .map_err(diesel::r2d2::Error::QueryError)?;
        }
        if self.enable_wal {
            conn.batch_execute("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
                .map_err(diesel::r2d2::Error::QueryError)?;
        }
        Ok(())
    }
}

pub fn create_pool(database_url: &str) -> DbPool {
    let manager = ConnectionManager::<SqliteConnection>::new(database_url);
    Pool::builder()
        .max_size(16)
        .min_idle(Some(1))
        .connection_customizer(Box::new(ConnectionOptions {
            enable_wal: true,
            busy_timeout: Some(Duration::from_secs(30)),
        }))
        .build(manager)
        .expect("Failed to create pool")
}