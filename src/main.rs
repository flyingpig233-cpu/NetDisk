mod api;
mod db;
mod db_types;
mod file_system;
mod jwt;
mod schema;
mod share;
mod user;
use file_system::file_manager::ensure_root;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    dotenv::dotenv().ok();
    jwt::secret_key();
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = db::create_pool(&database_url);

    {
        let mut conn = pool.get().expect("Failed to get database connection");
        ensure_root(&mut conn).expect("Error initializing root file");
        if let Ok(password) = std::env::var("ADMIN_PASSWORD") {
            if !password.is_empty() {
                user::ensure_admin(&mut conn, &password).expect("Error initializing admin account");
            }
        }
    }

    if std::env::args().any(|arg| arg == "--init-admin") {
        use diesel::prelude::*;
        let mut conn = pool.get().expect("Failed to get database connection");
        let count = schema::users::table
            .filter(schema::users::is_admin.eq(true))
            .count()
            .get_result::<i64>(&mut conn)
            .expect("Failed to check admin account");
        assert!(
            count > 0,
            "Set ADMIN_PASSWORD to initialize an admin account"
        );
        println!("Admin initialization completed");
        return;
    }
    let state = api::AppState { db: pool };
    let app = api::router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
