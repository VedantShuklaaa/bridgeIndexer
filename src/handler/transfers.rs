use axum::Json;
use axum::extract::{Query, State};

use crate::domain::api::TransferListResponse;
use crate::error::AppError;
use crate::services::transfers::{self, ListQuery};
use crate::state::AppState;

/// `GET /api/v2/transfers`. Validation lives in the service so the 400 messages are
/// stable and unit-tested; this handler only supplies the current chain snapshot.
pub async fn list_transfers_handler(
    State(state): State<AppState>,
    Query(raw): Query<ListQuery>,
) -> Result<Json<TransferListResponse>, AppError> {
    let chains = state.chains.snapshot();
    let response = transfers::list_transfers(&state.db, &chains, raw).await?;
    Ok(Json(response))
}
