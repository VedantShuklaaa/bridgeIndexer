use serde_json::{Value, json};

pub async fn get_transaction_data(
    client: &reqwest::Client,
    rpc_url: &str,
    tx_hash: &str,
) -> Result<Value, crate::error::AppError> {
    let receipt_body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "eth_getTransactionReceipt", "params": [tx_hash]
    });
    let receipt: Value = client
        .post(rpc_url)
        .json(&receipt_body)
        .send()
        .await?
        .json()
        .await?;

    let receipt_result = receipt
        .get("result")
        .cloned()
        .ok_or_else(|| crate::error::AppError::TransactionNotFound(tx_hash.to_string()))?;

    let block_number = receipt_result
        .get("blockNumber")
        .and_then(Value::as_str)
        .ok_or_else(|| crate::error::AppError::Normalisation("missing blockNumber".into()))?;

    let block_body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "eth_getBlockByNumber", "params": [block_number, false]
    });
    let block: Value = client
        .post(rpc_url)
        .json(&block_body)
        .send()
        .await?
        .json()
        .await?;
    let timestamp_hex = block
        .get("result")
        .and_then(|b| b.get("timestamp"))
        .and_then(Value::as_str);

    Ok(json!({ "receipt": receipt_result, "timestamp": timestamp_hex }))
}
