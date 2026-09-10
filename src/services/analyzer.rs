use crate::clients::{helius, wormhole};
use crate::domain::bridge_transfer::BridgeMessageId;
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::normaliser;
use crate::services::correlator::{self, CorrelateParams};
use crate::state::AppState;

pub async fn analyse_tx(state: &AppState, hash: &str) -> Result<NormalisedTransaction, AppError> {
    if hash.trim().is_empty() || hash.len() < 64 {
        return Err(AppError::InvalidTransactionHash(hash.to_string()));
    }

    let (solana_result, wormhole_result) = tokio::join!(
        helius::get_transaction(&state.http_client, &state.config.helius_url, hash),
        wormhole::get_operation_by_tx_hash(&state.http_client, &state.config.wormhole_url, hash),
    );

    let solana_raw = solana_result?;
    let mut tx = normaliser::solana::normalise(solana_raw, hash)?;
    tx.bridge_transfer = None;

    match wormhole_result {
        Ok(wh_raw) => {
            tracing::debug!(raw = %wh_raw, "wormhole raw response");
            let bridge_event = normaliser::wormhole::normalise(wh_raw.clone())?;

            let message_id = BridgeMessageId {
                emitter_chain: bridge_event.emitter_chain.parse().unwrap_or(1),
                emitter_address: bridge_event.emitter_address.clone(),
                sequence: bridge_event.sequence,
            };

            let destination_chain_id: u16 = bridge_event
                .target_chain
                .as_ref()
                .and_then(|c| c.parse().ok())
                .unwrap_or(0);

            let known_destination =
                normaliser::wormhole::extract_destination(&wh_raw).map(|(_, tx_hash)| {
                    correlator::KnownDestination {
                        tx_hash,
                        wallet: None,
                    }
                });

            tx.bridge_transfer = Some(
                correlator::correlate(
                    &state.registry,
                    CorrelateParams {
                        source_tx_hash: hash.to_string(),
                        source_wallet: tx.signer.clone(),
                        message_id,
                        destination_chain_id,
                        token: None,
                        amount: bridge_event.amount.clone(),
                        destination_wallet_from_vaa: None,
                        known_destination,
                    },
                )
                .await?,
            );

            tx.bridge_event = Some(bridge_event);
        }
        Err(AppError::TransactionNotFound(_)) => {
            tx.bridge_event = None;
        }
        Err(other) => return Err(other),
    }

    Ok(tx)
}

fn bridge_event_emitter(event: &crate::domain::bridge::BridgeEvent) -> String {
    event.emitter_address.clone()
}
