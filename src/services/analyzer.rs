use crate::clients::{helius, wormhole};
use crate::domain::bridge_transfer::{BridgeMessageId, ChainId};
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::normaliser;
use crate::services::correlator;
use crate::state::AppState;
use crate::vaa::decode::{decode_destination_address, decode_vaa};
use serde_json::Value;

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
            let bridge_event = normaliser::wormhole::normalise(wh_raw.clone())?;

            let vaa_b64 = wh_raw
                .get("vaa")
                .and_then(|v| v.get("raw"))
                .and_then(|r| r.as_str())
                .ok_or_else(|| AppError::Normalisation("missing vaa.raw".into()))?;

            let decoded = decode_vaa(vaa_b64)?;

            let message_id = BridgeMessageId {
                emitter_chain: decoded.header.emitter_chain,
                emitter_address: decoded.header.emitter_address.clone(),
                sequence: decoded.header.sequence,
            };

            let destination_chain_id: Option<u16> =
                decoded.transfer.as_ref().map(|t| t.to_chain).or_else(|| {
                    bridge_event
                        .target_chain
                        .as_ref()
                        .and_then(|c| c.parse().ok())
                });

            // NEW: raw u128, kept separate from the display string — needed
            // so the correlator can divide by the token's real decimals.
            // Only available when our own VAA decode recognized the payload;
            // Wormholescan's fallback amount is already decimal-formatted
            // text, not something we can safely re-parse as a raw integer.
            let raw_amount: Option<u128> = decoded.transfer.as_ref().map(|t| t.amount);

            let amount: Option<String> = decoded
                .transfer
                .as_ref()
                .map(|t| t.amount.to_string())
                .or_else(|| bridge_event.amount.clone());

            let token: Option<String> = decoded
                .transfer
                .as_ref()
                .and_then(|t| {
                    destination_chain_id.map(|c| decode_destination_address(c, &t.token_address))
                })
                .flatten()
                .or_else(|| extract_wormholescan_field(&wh_raw, "tokenAddress"));

            let destination_wallet: Option<String> = decoded
                .transfer
                .as_ref()
                .and_then(|t| {
                    destination_chain_id.map(|c| decode_destination_address(c, &t.to_address))
                })
                .flatten()
                .or_else(|| extract_wormholescan_field(&wh_raw, "toAddress"));

            let known_destination_tx: Option<String> = wh_raw
                .get("globalTx")
                .and_then(|g| g.get("destinationTx"))
                .and_then(|d| d.get("txHash"))
                .and_then(|h| h.as_str())
                .map(String::from);

            tx.bridge_transfer = Some(
                correlator::correlate(
                    &state.registry,
                    correlator::CorrelateParams {
                        source_tx_hash: hash.to_string(),
                        source_wallet: tx.signer.clone(),
                        message_id,
                        destination_chain_id: destination_chain_id.unwrap_or(0),
                        token,
                        raw_amount,
                        amount,
                        destination_wallet,
                        known_destination_tx,
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

fn extract_wormholescan_field(raw: &Value, field: &str) -> Option<String> {
    raw.get("content")
        .and_then(|c| c.get("standarizedProperties"))
        .and_then(|p| p.get(field))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}
