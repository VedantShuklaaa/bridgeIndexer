use reqwest::Client;
use serde_json::Value;

use crate::error::AppError;

/// Looks up a Wormhole VAA by Solana transaction signature.
/// A 404 here is expected and normal for non-bridge transactions —
/// callers should treat `TransactionNotFound` as "not a bridge tx",
/// not as a hard failure.
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

    // Response is a { "data": [...] } array — empty array means no VAA for this tx,
    // which is a normal "not a bridge tx" outcome, not an error.
    let vaas = payload.get("data").and_then(Value::as_array);
    match vaas {
        Some(arr) if !arr.is_empty() => Ok(arr[0].clone()),
        _ => Err(AppError::TransactionNotFound(hash.to_string())),
    }
}
