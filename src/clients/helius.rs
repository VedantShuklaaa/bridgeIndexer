use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;

/// Generic JSON-RPC 2.0 request envelope. Reusable for any Helius RPC
/// method (getTransaction, getSignaturesForAddress, etc.) — not tied
/// to getTransaction specifically.
#[derive(Debug, Serialize)]
struct JsonRpcRequest<T: Serialize> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: T,
}

/// The second element of Helius's getTransaction params array —
/// options controlling response shape.
#[derive(Debug, Serialize)]
struct GetTransactionOptions {
    encoding: &'static str,
    #[serde(rename = "maxSupportedTransactionVersion")]
    max_supported_transaction_version: u8,
}

impl JsonRpcRequest<(String, GetTransactionOptions)> {
    fn get_transaction(hash: &str) -> Self {
        Self {
            jsonrpc: "2.0",
            id: 1,
            method: "getTransaction",
            params: (
                hash.to_string(),
                GetTransactionOptions {
                    encoding: "jsonParsed",
                    max_supported_transaction_version: 0,
                },
            ),
        }
    }
}

pub async fn get_transaction(client: &Client, url: &str, hash: &str) -> Result<Value, AppError> {
    let body = JsonRpcRequest::get_transaction(hash);

    let resp = client.post(url).json(&body).send().await?;
    let payload: Value = resp.json().await?;

    if let Some(err) = payload.get("error") {
        return Err(AppError::UpstreamProvider {
            provider: "helius",
            message: err.to_string(),
        });
    }

    match payload.get("result") {
        Some(Value::Null) | None => Err(AppError::TransactionNotFound(hash.to_string())),
        Some(result) => Ok(result.clone()),
    }
}
