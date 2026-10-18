use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;
use reqwest::Client;
use serde_json::Value;

pub async fn get_operation_by_tx_hash(
    client: &Client,
    base_url: &str,
    hash: &str,
) -> Result<Value, AppError> {
    let url = format!("{base_url}/api/v1/operations?txHash={hash}");
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        return Err(AppError::UpstreamProvider {
            provider: "wormhole",
            message: format!("status {}", resp.status()),
        });
    }

    let payload: Value = resp.json().await?;
    let ops = payload.get("operations").and_then(Value::as_array); // was: payload.as_array()

    match ops {
        Some(arr) if !arr.is_empty() => Ok(arr[0].clone()),
        _ => Err(AppError::TransactionNotFound(hash.to_string())),
    }
}

pub async fn get_destination_tx(
    client: &Client,
    base_url: &str,
    id: &BridgeMessageId,
) -> Result<Option<String>, AppError> {
    let url = format!(
        "{base_url}/api/v1/global-tx/{}/{}/{}",
        id.emitter_chain, id.emitter_address, id.sequence
    );
    let resp = client.get(&url).send().await?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(AppError::UpstreamProvider {
            provider: "wormhole",
            message: format!("status {}", resp.status()),
        });
    }

    let payload: Value = resp.json().await?;
    Ok(destination_tx_from_global_tx(&payload))
}

/// Pure: reads `destinationTx.txHash` from a `global-tx` payload.
pub fn destination_tx_from_global_tx(payload: &Value) -> Option<String> {
    payload
        .get("destinationTx")
        .and_then(|d| d.get("txHash"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_destination_tx() {
        let p = json!({"destinationTx": {"txHash": "0xdest"}});
        assert_eq!(destination_tx_from_global_tx(&p).as_deref(), Some("0xdest"));
    }

    #[test]
    fn missing_or_empty_is_none() {
        for p in [
            json!({}),
            json!({"destinationTx": {}}),
            json!({"destinationTx": {"txHash": ""}}),
            json!({"destinationTx": {"txHash": "  "}}),
        ] {
            assert_eq!(destination_tx_from_global_tx(&p), None);
        }
    }
}
