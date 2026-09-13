use crate::clients::{evm, helius, wormhole};
use crate::domain::bridge_transfer::BridgeMessageId;
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::normaliser;
use crate::services::correlator;
use crate::state::AppState;
use crate::vaa::decode::{decode_destination_address, decode_vaa};
use serde_json::Value;

pub async fn analyse_tx(
    state: &AppState,
    chain: &str,
    hash: &str,
) -> Result<NormalisedTransaction, AppError> {
    if hash.trim().is_empty() {
        return Err(AppError::InvalidTransactionHash(hash.to_string()));
    }

    let (source_result, wormhole_result) = tokio::join!(
        fetch_source_transaction(state, chain, hash),
        wormhole::get_operation_by_tx_hash(&state.http_client, &state.config.wormhole_url, hash),
    );

    let mut tx = source_result?;
    tx.bridge_transfer = None;

    match wormhole_result {
        Ok(wh_raw) => {
            let bridge_event = normaliser::wormhole::normalise(wh_raw.clone())?;

            let vaa_b64 = wh_raw
                .get("vaa")
                .and_then(|v| v.get("raw"))
                .and_then(|r| r.as_str())
                .ok_or_else(|| AppError::VaaNotAvailable(hash.to_string()))?;

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

            fn extract_wormholescan_number_field(raw: &Value, field: &str) -> Option<u16> {
                raw.get("content")
                    .and_then(|c| c.get("standarizedProperties"))
                    .and_then(|p| p.get(field))
                    .and_then(Value::as_u64)
                    .map(|n| n as u16)
            }

            let token_chain: Option<u16> = decoded
                .transfer
                .as_ref()
                .map(|t| t.token_chain)
                .or_else(|| extract_wormholescan_number_field(&wh_raw, "tokenChain"));

            let raw_amount: Option<u128> = decoded.transfer.as_ref().map(|t| t.amount);

            let amount: Option<String> = decoded
                .transfer
                .as_ref()
                .map(|t| t.amount.to_string())
                .or_else(|| bridge_event.amount.clone());

            let token: Option<String> = decoded
                .transfer
                .as_ref()
                .and_then(|t| token_chain.map(|c| decode_destination_address(c, &t.token_address)))
                //                ^^^^^^^^^^^ was destination_chain_id — token_address decodes per token_chain, not destination
                .flatten()
                .or_else(|| extract_wormholescan_field(&wh_raw, "tokenAddress"));

            tracing::info!(
                "WormholeScan standardizedProperties: {:?}",
                wh_raw
                    .get("content")
                    .and_then(|c| c.get("standarizedProperties"))
            );

            let wormholescan_symbol_hint: Option<String> =
                extract_wormholescan_field(&wh_raw, "tokenSymbol");

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
                        token_chain: token_chain.unwrap_or(0),
                        wormholescan_symbol_hint,
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

async fn fetch_source_transaction(
    state: &AppState,
    chain: &str,
    hash: &str,
) -> Result<NormalisedTransaction, AppError> {
    match chain {
        "solana" => {
            let raw =
                helius::get_transaction(&state.http_client, &state.config.helius_url, hash).await?;
            normaliser::solana::normalise(raw, hash)
        }
        evm_chain @ ("ethereum" | "bsc" | "polygon" | "avalanche" | "arbitrum" | "optimism"
        | "gnosis" | "base") => {
            let rpc_url = state
                .config
                .rpc_url_for_chain(evm_chain)
                .map_err(|e| AppError::Normalisation(e.to_string()))?;
            let raw = evm::get_transaction_data(&state.http_client, rpc_url, hash).await?;
            normaliser::evm::normalise(raw, hash, evm_chain)
        }
        other => Err(AppError::Normalisation(format!(
            "unsupported chain: {other}"
        ))),
    }
}
