use serde_json::Value;

use crate::domain::transaction::{Chain, NormalisedTransaction, TxStatus};
use crate::error::AppError;

fn parse_hex_u64(value: &Value) -> Option<u64> {
    value
        .as_str()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
}

pub fn normalise(
    raw: Value,
    hash: &str,
    chain_label: &str,
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
    })
}
