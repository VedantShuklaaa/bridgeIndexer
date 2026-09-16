use alloy_json_abi::JsonAbi;
use serde_json::Value;

use crate::decoding::evm_decode::decode_transaction;
use crate::domain::transaction::{Chain, NormalisedTransaction, TxStatus};
use crate::error::AppError;

fn parse_hex_u64(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
}

/// Normalises raw RPC data (`{ transaction, receipt, timestamp }`, as
/// produced by `clients::evm::get_transaction_data`) into a `NormalisedTransaction`.
///
/// `abi`, if supplied, is passed straight through to the generic decoder to
/// decode arbitrary contract calldata against `transaction.to`. This is
/// entirely optional — standard ERC-20/721/1155 log decoding, and native
/// value transfers, work without any ABI.
pub fn normalise(
    raw: Value,
    hash: &str,
    chain_label: &str,
    abi: Option<&JsonAbi>,
) -> Result<NormalisedTransaction, AppError> {
    let chain = Chain::from_label(chain_label)
        .ok_or_else(|| AppError::Normalisation(format!("unknown EVM chain: {chain_label}")))?;

    let receipt = raw
        .get("receipt")
        .ok_or_else(|| AppError::Normalisation("missing receipt".into()))?;

    let slot = receipt
        .get("blockNumber")
        .and_then(parse_hex_u64)
        .ok_or_else(|| AppError::Normalisation("missing blockNumber".into()))?;

    let status = match receipt.get("status").and_then(parse_hex_u64) {
        Some(1) => TxStatus::Success,
        _ => TxStatus::Failed,
    };

    let gas_used = receipt.get("gasUsed").and_then(parse_hex_u64).unwrap_or(0);
    let gas_price = receipt
        .get("effectiveGasPrice")
        .and_then(parse_hex_u64)
        .unwrap_or(0);
    let fee_lamports = gas_used.saturating_mul(gas_price);

    let signer = receipt
        .get("from")
        .and_then(Value::as_str)
        .map(String::from);

    let timestamp = raw
        .get("timestamp")
        .and_then(parse_hex_u64)
        .map(|t| t as i64);

    // Generic, chain-agnostic decoding: native value, calldata (if `abi`
    // matches), and standard ERC-20/721/1155 logs. Never fails the whole
    // normalisation — unrecognised pieces come back as DecodedAction::Unknown.
    let decoded_actions = decode_transaction(&raw, abi)?;

    Ok(NormalisedTransaction {
        hash: hash.to_string(),
        chain,
        status,
        slot,
        timestamp,
        fee_lamports,
        signer,
        bridge_event: None,
        bridge_transfer: None,
        decoded_actions,
    })
}
