use axum::Router;
use axum::routing::get;

use crate::handler::analyse::analyse_v2_handler;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/analyse", get(analyse_v2_handler))
}
