use axum::Router;
use axum::routing::get;

use crate::handler::analyse::analyse_handler;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/analyse", get(analyse_handler))
        .with_state(state)
}
