use crate::clients::limiter::call_upstream;
use crate::clients::{helius, wormhole};
use crate::domain::bridge_transfer::{BridgeMessageId, BridgeStatus};
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::normaliser;
use crate::protocols::SolanaTx;
use crate::services::correlator;
use crate::services::detect::{self, Enrichment};
use crate::services::token_metadata::resolve_token_metadata;
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

    let t_total = std::time::Instant::now();
    let (source_result, wormhole_result) = tokio::join!(
        async {
            let t = std::time::Instant::now();
            let r = fetch_source_transaction(state, chain, hash).await;
            tracing::info!(helius_ms = t.elapsed().as_millis() as u64, "source fetch");
            r
        },
        async {
            let t = std::time::Instant::now();
            let r = call_upstream(&state.wormhole_limiter, || {
                wormhole::get_operation_by_tx_hash(
                    &state.http_client,
                    &state.config.wormhole_url,
                    hash,
                )
            })
            .await;
            tracing::info!(wormhole_ms = t.elapsed().as_millis() as u64, "wormholescan");
            r
        },
    );

    let (mut tx, solana_tx) = source_result?;
    tx.bridge_transfer = None;

    // Chain first: if an adapter decodes a transfer from the Solana tx alone, it is
    // stored as `Detected` whatever WormholeScan says. WormholeScan only enriches.
    if let Some(solana_tx) = &solana_tx {
        let detected = detect::detect(&state.protocols, solana_tx);
        if let Some(first) = detected.first() {
            if detected.len() > 1 {
                tracing::warn!(
                    signature = %hash,
                    count = detected.len(),
                    "several transfers in one tx, only the first is stored until the v2 model"
                );
            }

            // Best effort: a slow or failing metadata lookup never blocks detection.
            let metadata = match detect::token_lookup(first) {
                Some((token_chain, address)) => tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    resolve_token_metadata(&state.registry, token_chain, &address, None),
                )
                .await
                .ok()
                .and_then(Result::ok),
                None => None,
            };

            let mut transfer = detect::to_bridge_transfer(first, metadata)?;
            match &wormhole_result {
                Ok(wh_raw) => match detect::apply_wormholescan(&mut transfer, wh_raw) {
                    Enrichment::Applied => {
                        tx.bridge_event = normaliser::wormhole::normalise(wh_raw.clone()).ok();
                        // VAA matched but /operations carried no destination tx:
                        // ask global-tx, keyed by the on-chain message id.
                        if matches!(transfer.status, BridgeStatus::Pending)
                            && let Some(hash) =
                                global_tx_destination(state, &transfer.message_id).await
                        {
                            detect::apply_destination_tx(&mut transfer, &hash);
                        }
                    }
                    outcome => {
                        tracing::debug!(signature = %hash, ?outcome, "wormholescan not applied")
                    }
                },
                Err(error) => {
                    tracing::debug!(signature = %hash, %error, "wormholescan unavailable, keeping on-chain data")
                }
            }
            tx.bridge_transfer = Some(transfer);

            tracing::info!(
                total_ms = t_total.elapsed().as_millis() as u64,
                "analyse_tx total (chain-first)"
            );
            return Ok(tx);
        }
    }

    // No adapter recognised the tx (inbound redeem, another Wormhole program, ...):
    // keep the WormholeScan-driven path until those adapters exist.
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

            let known_destination_tx: Option<String> =
                match detect::destination_tx_from_operation(&wh_raw) {
                    Some(hash) => Some(hash),
                    None => global_tx_destination(state, &message_id).await,
                };

            let t_corr = std::time::Instant::now();
            let correlated = correlator::correlate(
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
            .await?;
            tracing::info!(
                correlate_ms = t_corr.elapsed().as_millis() as u64,
                "correlate"
            );
            tx.bridge_transfer = Some(correlated);

            tx.bridge_event = Some(bridge_event);
        }

        Err(other) => return Err(other),
    }

    tracing::info!(
        total_ms = t_total.elapsed().as_millis() as u64,
        "analyse_tx total"
    );
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

/// Best-effort `global-tx` lookup through the WormholeScan limiter. A failure is
/// logged and treated as "not known yet", it never fails the analysis.
async fn global_tx_destination(state: &AppState, id: &BridgeMessageId) -> Option<String> {
    call_upstream(&state.wormhole_limiter, || {
        wormhole::get_destination_tx(&state.http_client, &state.config.wormhole_url, id)
    })
    .await
    .inspect_err(|error| tracing::debug!(%error, "global-tx lookup failed"))
    .ok()
    .flatten()
}

/// The normalised tx, plus the pure `SolanaTx` view adapters read (`None` if the raw
/// response could not be viewed, in which case only the WormholeScan path can run).
async fn fetch_source_transaction(
    state: &AppState,
    chain: &str,
    hash: &str,
) -> Result<(NormalisedTransaction, Option<SolanaTx>), AppError> {
    match chain {
        "solana" => {
            let raw = call_upstream(&state.helius_limiter, || {
                helius::get_transaction(&state.http_client, &state.config.helius_url, hash)
            })
            .await?;
            let solana_tx = SolanaTx::from_helius(&raw, hash)
                .inspect_err(|error| tracing::warn!(signature = %hash, %error, "unreadable tx"))
                .ok();
            Ok((normaliser::solana::normalise(raw, hash)?, solana_tx))
        }
        other => Err(AppError::Normalisation(format!(
            "unsupported chain: {other}"
        ))),
    }
}
