use reqwest::Client;
use serde_json::Value;

use crate::error::AppError;

pub async fn get_vaa_by_tx_hash(
    client: &Client,
    base_url: &str,
    hash: &str,
) -> Result<Value, AppError> {
    let url = format!("{base_url}/api/v1/vaas?txHash={hash}");

    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        return Err(AppError::UpstreamProvider {
            provider: "wormhole",
            message: format!("status {}", resp.status()),
        });
    }

    let payload: Value = resp.json().await?;

    let vaas = payload.get("data").and_then(Value::as_array);
    match vaas {
        Some(arr) if !arr.is_empty() => Ok(arr[0].clone()),
        _ => Err(AppError::TransactionNotFound(hash.to_string())),
    }
}

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
