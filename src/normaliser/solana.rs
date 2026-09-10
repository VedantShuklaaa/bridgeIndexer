use serde_json::Value;

use crate::domain::transaction::{Chain, NormalisedTransaction, TxStatus};
use crate::error::AppError;

/// Pure transform: raw Helius `jsonParsed` result -> our domain type.
/// No network calls, unit-testable with a saved fixture.
pub fn normalise(raw: Value, hash: &str) -> Result<NormalisedTransaction, AppError> {
    let slot = raw
        .get("slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::Normalisation("missing slot".into()))?;

    let timestamp = raw.get("blockTime").and_then(Value::as_i64);

    let status = if raw
        .get("meta")
        .and_then(|m| m.get("err"))
        .map(|e| !e.is_null())
        .unwrap_or(false)
    {
        TxStatus::Failed
    } else {
        TxStatus::Success
    };

    let fee_lamports = raw
        .get("meta")
        .and_then(|m| m.get("fee"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let signer = raw
        .get("transaction")
        .and_then(|t| t.get("message"))
        .and_then(|m| m.get("accountKeys"))
        .and_then(Value::as_array)
        .and_then(|keys| keys.first())
        .and_then(|k| k.get("pubkey").or(Some(k)))
        .and_then(Value::as_str)
        .map(String::from);

    Ok(NormalisedTransaction {
        hash: hash.to_string(),
        chain: Chain::Solana,
        status,
        slot,
        timestamp,
        fee_lamports,
        signer,
        bridge_event: None, 
        bridge_transfer: None, 
    })
}
