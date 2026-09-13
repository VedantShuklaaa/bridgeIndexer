use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;

use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::services::analyzer;
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
    let tx = analyzer::analyse_tx(&state, &params.chain, &params.transaction_hash).await?;
    Ok(Json(tx))
}
