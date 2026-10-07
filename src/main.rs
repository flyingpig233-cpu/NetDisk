mod api;
mod db;
mod db_types;
mod file_system;
mod schema;
mod jwt;
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
    }

    let state = api::AppState { db: pool };
    let app = api::router(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}
