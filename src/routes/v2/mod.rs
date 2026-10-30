mod transactions;

use axum::Router;
use axum::routing::get;

use crate::handler::transfers::list_transfers_handler;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/transactions", transactions::router())
        .route("/transfers", get(list_transfers_handler))
}
