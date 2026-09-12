use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;

#[derive(Debug, Serialize)]
struct JsonRpcRequest<T: Serialize> {
    jsonrpc: &'static str,
    id: u64,
    method: &'static str,
    params: T,
}

#[derive(Debug, Serialize)]
struct GetSignaturesForAddressOptions {
    commitment: &'static str,

    #[serde(skip_serializing_if = "Option::is_none")]
    before: Option<String>,
    limit: u64,
}

#[derive(Debug, Serialize)]
struct GetTransactionOptions {
    encoding: &'static str,
    #[serde(rename = "maxSupportedTransactionVersion")]
    max_supported_transaction_version: u8,
    commitment: &'static str,
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
                    commitment: "confirmed",
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

pub async fn get_signatures_for_address(
    client: &Client,
    url: &str,
    address: &str,
    before: Option<String>,
) -> Result<Vec<Value>, AppError> {
    let body = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "getSignaturesForAddress",
        params: (
            address.to_string(),
            GetSignaturesForAddressOptions {
                commitment: "confirmed",
                before,
                limit: 1000,
            },
        ),
    };

    let resp = client.post(url).json(&body).send().await?;

    let payload: Value = resp.json().await?;

    if let Some(err) = payload.get("error") {
        return Err(AppError::UpstreamProvider {
            provider: "helius",
            message: err.to_string(),
        });
    }

    match payload.get("result") {
        Some(Value::Array(result)) => Ok(result.clone()),

        _ => Err(AppError::UpstreamProvider {
            provider: "helius",
            message: "invalid getSignaturesForAddress response".to_string(),
        }),
    }
}
