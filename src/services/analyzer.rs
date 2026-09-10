use crate::clients::{helius, wormhole};
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::normaliser;
use crate::state::AppState;

pub async fn analyse_tx(state: &AppState, hash: &str) -> Result<NormalisedTransaction, AppError> {
    if hash.trim().is_empty() || hash.len() < 64 {
        return Err(AppError::InvalidTransactionHash(hash.to_string()));
    }

    let (solana_result, wormhole_result) = tokio::join!(
        helius::get_transaction(&state.http_client, &state.config.helius_url, hash),
        wormhole::get_vaa_by_tx_hash(&state.http_client, &state.config.wormhole_url, hash),
    );

    // Solana lookup failing is a hard error — no transaction, nothing to return.
    let solana_raw = solana_result?;
    let mut tx = normaliser::solana::normalise(solana_raw, hash)?;

    // Wormhole "not found" just means this isn't a bridge tx — not a failure.
    match wormhole_result {
        Ok(wh_raw) => {
            tx.bridge_event = Some(normaliser::wormhole::normalise(wh_raw)?);
        }
        Err(AppError::TransactionNotFound(_)) => {
            tx.bridge_event = None;
        }
        Err(other) => return Err(other),
    }

    Ok(tx)
}
