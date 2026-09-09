use axum::{Router, routing::get, serve};
use std::env;
use tokio::net::TcpListener;

use crate::{
    config::config::{HOST, PORT},
    db::connection::connect_db,
    state::state::AppState,
};

mod config;
mod db;
mod state;
mod handler;
mod error;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let db = connect_db(&database_url).await;

    let state = AppState { db };

    let app = Router::new().route("/", get(hello)).with_state(state);

    let addr = format!("{}:{}", HOST, PORT);
    let listener = TcpListener::bind(&addr).await.unwrap();

    serve(listener, app).await.unwrap();
}

async fn hello() -> &'static str {
    "hello world"
}
