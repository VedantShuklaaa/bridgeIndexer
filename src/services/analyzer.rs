use crate::clients::{evm, helius, wormhole};
use crate::domain::bridge::BridgeEvent;
use crate::domain::bridge_transfer::{BridgeMessageId, BridgeTransfer};
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::normaliser;
use crate::services::correlator;
use crate::state::AppState;
use crate::vaa::decode::{decode_destination_address, decode_transfer_if_present};
use serde_json::Value;

pub async fn analyse_tx(
    state: &AppState,
    chain: &str,
    hash: &str,
) -> Result<NormalisedTransaction, AppError> {
    if hash.trim().is_empty() {
        return Err(AppError::InvalidTransactionHash(hash.to_string()));
    }

    let (mut tx, bridge_event, bridge_transfer) = match chain {
        "solana" => {
            let raw =
                helius::get_transaction(&state.http_client, &state.config.helius_url, hash)
                    .await?;
            let tx = normaliser::solana::normalise(raw, hash)?;
            let (bridge_event, bridge_transfer) =
                build_bridge_data_via_wormholescan(state, &tx, hash).await?;
            (tx, bridge_event, bridge_transfer)
        }
        evm_chain @ ("ethereum" | "bsc" | "polygon" | "avalanche" | "arbitrum" | "optimism"
        | "gnosis" | "base") => {
            let rpc_url = state
                .config
                .rpc_url_for_chain(evm_chain)
                .map_err(|e| AppError::Normalisation(e.to_string()))?;
            // Fetched once and reused for both normalisation and log
            // extraction below — no repeat RPC round-trip.
            let raw = evm::get_transaction_data(&state.http_client, rpc_url, hash).await?;
            let tx = normaliser::evm::normalise(raw.clone(), hash, evm_chain)?;
            let (bridge_event, bridge_transfer) =
                build_bridge_data_from_source_receipt(state, &tx, evm_chain, hash, &raw).await?;
            (tx, bridge_event, bridge_transfer)
        }
        other => {
            return Err(AppError::Normalisation(format!(
                "unsupported chain: {other}"
            )));
        }
    };

    tx.bridge_event = Some(bridge_event);
    tx.bridge_transfer = Some(bridge_transfer);

    Ok(tx)
}

/// EVM path: the transfer is decoded entirely from the source transaction's
/// own receipt — the Core Bridge's `LogMessagePublished` event. No
/// WormholeScan call, no guardian VAA fetch; the payload bytes are
/// identical either way, we just read them a step earlier.
async fn build_bridge_data_from_source_receipt(
    state: &AppState,
    tx: &NormalisedTransaction,
    chain: &str,
    hash: &str,
    raw: &Value,
) -> Result<(BridgeEvent, BridgeTransfer), AppError> {
    let core_bridge_contract = state
        .config
        .core_bridge_contract_for_chain(chain)
        .map_err(|e| AppError::Normalisation(e.to_string()))?;

    let receipt = raw
        .get("receipt")
        .ok_or_else(|| AppError::Normalisation("missing receipt".into()))?;

    let own_wormhole_chain_id = crate::domain::transaction::Chain::from_label(chain)
        .ok_or_else(|| AppError::Normalisation(format!("unknown EVM chain: {chain}")))?
        .wormhole_id();

    let msg = crate::vaa::evm_log::extract_source_message(
        receipt,
        core_bridge_contract,
        own_wormhole_chain_id,
    )?;

    let transfer = decode_transfer_if_present(&msg.payload)?;

    let message_id = BridgeMessageId {
        emitter_chain: msg.emitter_chain,
        emitter_address: msg.emitter_address.clone(),
        sequence: msg.sequence,
    };

    let destination_chain_id = transfer.as_ref().map(|t| t.to_chain);
    let token_chain = transfer.as_ref().map(|t| t.token_chain);
    let raw_amount = transfer.as_ref().map(|t| t.amount);
    let amount = transfer.as_ref().map(|t| t.amount.to_string());

    let token = transfer
        .as_ref()
        .and_then(|t| token_chain.map(|c| decode_destination_address(c, &t.token_address)))
        .flatten();

    let destination_wallet = transfer
        .as_ref()
        .and_then(|t| destination_chain_id.map(|c| decode_destination_address(c, &t.to_address)))
        .flatten();

    let bridge_event = BridgeEvent {
        sequence: msg.sequence,
        emitter_chain: msg.emitter_chain.to_string(),
        emitter_address: msg.emitter_address,
        target_chain: destination_chain_id.map(|c| c.to_string()),
        amount: amount.clone(),
    };

    let bridge_transfer = correlator::correlate(
        &state.registry,
        correlator::CorrelateParams {
            source_tx_hash: hash.to_string(),
            source_wallet: tx.signer.clone(),
            message_id,
            destination_chain_id: destination_chain_id.unwrap_or(0),
            token,
            token_chain: token_chain.unwrap_or(0),
            symbol_hint: None, // no external hint anymore; resolve_token_metadata falls back to on-chain reads only
            raw_amount,
            amount,
            destination_wallet,
            known_destination_tx: None, // always look this up ourselves now
        },
    )
    .await?;

    Ok((bridge_event, bridge_transfer))
}

/// Solana path — TEMPORARY. Still goes through WormholeScan for the VAA.
///
/// Extracting the same data straight from a Solana transaction means
/// decoding the token bridge program's own instruction layout (Borsh), and
/// getting those byte offsets wrong silently corrupts amounts/addresses
/// rather than erroring — not something to guess at from memory. Left
/// as-is on purpose until it can be verified against the actual program
/// IDL, rather than shipping an EVM-only fix and pretending Solana is
/// covered too.
async fn build_bridge_data_via_wormholescan(
    state: &AppState,
    tx: &NormalisedTransaction,
    hash: &str,
) -> Result<(BridgeEvent, BridgeTransfer), AppError> {
    let wh_raw =
        wormhole::get_operation_by_tx_hash(&state.http_client, &state.config.wormhole_url, hash)
            .await?;

    let bridge_event = normaliser::wormhole::normalise(wh_raw.clone())?;

    let vaa_b64 = wh_raw
        .get("vaa")
        .and_then(|v| v.get("raw"))
        .and_then(|r| r.as_str())
        .ok_or_else(|| AppError::VaaNotAvailable(hash.to_string()))?;

    let decoded = crate::vaa::decode::decode_vaa(vaa_b64)?;

    let message_id = BridgeMessageId {
        emitter_chain: decoded.header.emitter_chain,
        emitter_address: decoded.header.emitter_address.clone(),
        sequence: decoded.header.sequence,
    };

    let destination_chain_id: Option<u16> = decoded
        .transfer
        .as_ref()
        .map(|t| t.to_chain)
        .or_else(|| {
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

    fn extract_wormholescan_field(raw: &Value, field: &str) -> Option<String> {
        raw.get("content")
            .and_then(|c| c.get("standarizedProperties"))
            .and_then(|p| p.get(field))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
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
        .flatten()
        .or_else(|| extract_wormholescan_field(&wh_raw, "tokenAddress"));

    let symbol_hint: Option<String> = extract_wormholescan_field(&wh_raw, "tokenSymbol");

    let destination_wallet: Option<String> = decoded
        .transfer
        .as_ref()
        .and_then(|t| destination_chain_id.map(|c| decode_destination_address(c, &t.to_address)))
        .flatten()
        .or_else(|| extract_wormholescan_field(&wh_raw, "toAddress"));

    let known_destination_tx: Option<String> = wh_raw
        .get("globalTx")
        .and_then(|g| g.get("destinationTx"))
        .and_then(|d| d.get("txHash"))
        .and_then(|h| h.as_str())
        .map(String::from);

    let bridge_transfer = correlator::correlate(
        &state.registry,
        correlator::CorrelateParams {
            source_tx_hash: hash.to_string(),
            source_wallet: tx.signer.clone(),
            message_id,
            destination_chain_id: destination_chain_id.unwrap_or(0),
            token,
            token_chain: token_chain.unwrap_or(0),
            symbol_hint,
            raw_amount,
            amount,
            destination_wallet,
            known_destination_tx,
        },
    )
    .await?;

    Ok((bridge_event, bridge_transfer))
}
