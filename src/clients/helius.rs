use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use crate::error::AppError;
use crate::protocols::AccountData;

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
struct GetMultipleAccountsOptions {
    encoding: &'static str,
    commitment: &'static str,
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
                    max_supported_transaction_version: 1,
                    commitment: "confirmed",
                },
            ),
        }
    }
}

pub async fn get_transaction(client: &Client, url: &str, hash: &str) -> Result<Value, AppError> {
    let body = JsonRpcRequest::get_transaction(hash);

    let resp = client.post(url).json(&body).send().await?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AppError::UpstreamRateLimited("helius"));
    }

    if !resp.status().is_success() {
        return Err(AppError::UpstreamProvider {
            provider: "helius",
            message: format!("status {}", resp.status()),
        });
    }

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

/// `getMultipleAccounts` (base64). One entry per requested key, in order: `None` for
/// an account that does not exist. The RPC accepts at most 100 keys per call.
pub async fn get_multiple_accounts(
    client: &Client,
    url: &str,
    keys: &[String],
) -> Result<Vec<Option<AccountData>>, AppError> {
    let body = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "getMultipleAccounts",
        params: (
            keys,
            GetMultipleAccountsOptions {
                encoding: "base64",
                commitment: "confirmed",
            },
        ),
    };

    let resp = client.post(url).json(&body).send().await?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AppError::UpstreamRateLimited("helius"));
    }

    if !resp.status().is_success() {
        return Err(AppError::UpstreamProvider {
            provider: "helius",
            message: format!("status {}", resp.status()),
        });
    }

    let payload: Value = resp.json().await?;

    if let Some(err) = payload.get("error") {
        return Err(AppError::UpstreamProvider {
            provider: "helius",
            message: err.to_string(),
        });
    }

    parse_multiple_accounts(&payload, keys.len())
}

fn parse_multiple_accounts(
    payload: &Value,
    expected: usize,
) -> Result<Vec<Option<AccountData>>, AppError> {
    let bad = |message: &str| AppError::UpstreamProvider {
        provider: "helius",
        message: message.to_string(),
    };
    let values = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .filter(|v| v.len() == expected)
        .ok_or_else(|| bad("getMultipleAccounts: unexpected result shape"))?;

    values
        .iter()
        .map(|v| {
            if v.is_null() {
                return Ok(None);
            }
            let owner = v
                .get("owner")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("getMultipleAccounts: account without owner"))?;
            let data = v
                .pointer("/data/0")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("getMultipleAccounts: account without data"))?;
            let data = STANDARD
                .decode(data)
                .map_err(|_| bad("getMultipleAccounts: data is not base64"))?;
            Ok(Some(AccountData {
                owner: owner.to_string(),
                data,
            }))
        })
        .collect()
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

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AppError::UpstreamRateLimited("helius"));
    }

    if !resp.status().is_success() {
        return Err(AppError::UpstreamProvider {
            provider: "helius",
            message: format!("status {}", resp.status()),
        });
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_accounts_and_missing_entries_in_order() {
        let payload = json!({ "result": { "value": [
            { "owner": "Owner", "data": ["aGk=", "base64"] },
            null
        ]}});
        let out = parse_multiple_accounts(&payload, 2).unwrap();
        assert_eq!(
            out[0],
            Some(AccountData {
                owner: "Owner".into(),
                data: b"hi".to_vec()
            })
        );
        assert_eq!(out[1], None);
    }

    #[test]
    fn a_wrong_count_or_bad_data_is_an_error() {
        let payload = json!({ "result": { "value": [null] } });
        assert!(parse_multiple_accounts(&payload, 2).is_err());
        let bad = json!({ "result": { "value": [{ "owner": "O", "data": ["!!", "base64"] }] } });
        assert!(parse_multiple_accounts(&bad, 1).is_err());
    }
}
