use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::domain::api::TransactionResponse;
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::routes::v1::compat;
use crate::services::read_through;
use crate::state::AppState;

fn default_chain() -> String {
    "solana".to_string()
}

#[derive(Deserialize)]
pub struct AnalyseQuery {
    pub transaction_hash: String,
    #[serde(default = "default_chain")]
    pub chain: String,
}

pub async fn analyse_handler(
    State(state): State<AppState>,
    Query(params): Query<AnalyseQuery>,
) -> Result<Json<NormalisedTransaction>, AppError> {
    let analysed =
        read_through::analyse_cached_v2(&state, &params.chain, &params.transaction_hash).await?;
    Ok(Json(compat::to_v1(
        &analysed.response,
        analysed.bridge_event,
    )?))
}

pub async fn analyse_v2_handler(
    State(state): State<AppState>,
    Query(params): Query<AnalyseQuery>,
) -> Result<Json<TransactionResponse>, AppError> {
    let analysed =
        read_through::analyse_cached_v2(&state, &params.chain, &params.transaction_hash).await?;
    Ok(Json(analysed.response))
}
