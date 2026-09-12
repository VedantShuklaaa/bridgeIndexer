use axum::Router;
use axum::routing::get;

use crate::handler::analyse::analyse_handler;
use crate::handler::stream::stream_ws_handler;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/analyse", get(analyse_handler))
        .route("/stream", get(stream_ws_handler))
}
