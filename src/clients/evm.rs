use serde_json::{Value, json};

use crate::error::AppError;

// ── RPC plumbing ──────────────────────────────────────────────────────────────

async fn rpc_call(
    client: &reqwest::Client,
    provider: &str,
    rpc_url: &str,
    body: &Value,
) -> Result<Value, AppError> {
    let resp = client.post(rpc_url).json(body).send().await?;
    let status = resp.status();
    let text = resp.text().await?;

    if status.as_u16() == 429 {
        return Err(AppError::UpstreamRateLimited(provider.to_string()));
    }

    if !status.is_success() {
        return Err(AppError::upstream(
            provider,
            format!("http {status}: {:.300}", text),
        ));
    }

    let parsed: Value = serde_json::from_str(&text)
        .map_err(|e| AppError::upstream(provider, format!("non-json body ({e}): {:.300}", text)))?;

    if let Some(err) = parsed.get("error").filter(|v| !v.is_null()) {
        return Err(AppError::upstream(provider, format!("rpc error: {err}")));
    }

    Ok(parsed)
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Fetches everything the generic EVM decoder needs for one transaction:
/// the transaction itself (to/input/value/from), its receipt (status,
/// gasUsed, logs), and the block timestamp. Chain-agnostic — caller supplies
/// the RPC URL for whichever EVM chain the tx lives on.
///
/// Returns:
/// { "transaction": <eth_getTransactionByHash result>,
///   "receipt": <eth_getTransactionReceipt result>,
///   "timestamp": <hex string from the block, or null> }
pub async fn get_transaction_data(
    client: &reqwest::Client,
    rpc_url: &str,
    provider: &str,
    tx_hash: &str,
) -> Result<Value, AppError> {
    let tx_response = rpc_call(
        client,
        provider,
        rpc_url,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionByHash",
            "params": [tx_hash],
        }),
    )
    .await?;

    let transaction = tx_response
        .get("result")
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or_else(|| AppError::TransactionNotFound(tx_hash.to_string()))?;

    let receipt = rpc_call(
        client,
        provider,
        rpc_url,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getTransactionReceipt",
            "params": [tx_hash],
        }),
    )
    .await?;

    // A mined tx can still have a null receipt if it's not yet included;
    // eth_getTransactionByHash succeeding doesn't guarantee this.
    let receipt_result = receipt
        .get("result")
        .filter(|v| !v.is_null())
        .cloned()
        .ok_or_else(|| AppError::TransactionNotFound(tx_hash.to_string()))?;

    let block_number = receipt_result
        .get("blockNumber")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Normalisation("missing blockNumber".into()))?;

    let block = rpc_call(
        client,
        provider,
        rpc_url,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getBlockByNumber",
            "params": [block_number, false],
        }),
    )
    .await?;

    let timestamp = block
        .get("result")
        .and_then(|b| b.get("timestamp"))
        .and_then(Value::as_str);

    Ok(json!({
        "transaction": transaction,
        "receipt": receipt_result,
        "timestamp": timestamp,
    }))
}
