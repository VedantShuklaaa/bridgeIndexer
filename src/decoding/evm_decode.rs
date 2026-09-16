//! Generic, chain-agnostic EVM transaction decoder.
//!
//! This module knows nothing about which EVM chain a transaction came from —
//! it only sees the raw `{ transaction, receipt, timestamp }` JSON produced
//! by `evm_rpc::get_transaction_data` and decodes it structurally, based on
//! the `to` address, calldata (`input`), and emitted log topics. The same
//! logic runs for Ethereum, Base, Optimism, Arbitrum, Polygon, BSC,
//! Avalanche, or any other EVM-compatible chain — there is no per-chain
//! branch anywhere in this file.
//!
//! Decoding never fails the whole transaction: anything we can't recognise
//! (an unknown log topic, calldata with no matching ABI) becomes
//! `DecodedAction::Unknown` rather than an error, so `decode_transaction`
//! always returns a result as long as the input JSON has the expected shape.

use alloy_dyn_abi::{DynSolValue, EventExt, FunctionExt, JsonAbiExt};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, B256, U256};
use serde::Serialize;
use serde_json::Value;
use sha3::{Digest, Keccak256};
use std::str::FromStr;

use crate::error::AppError;

// ── Well-known event topics (keccak256 of the event signature) ────────────────

fn topic0(signature: &str) -> B256 {
    B256::from_slice(&Keccak256::digest(signature.as_bytes()))
}

/// `Transfer(address,address,uint256)` — shared by ERC-20 and ERC-721; the
/// two are told apart by whether the third topic (tokenId) is indexed,
/// which shows up as 4 topics total for ERC-721 vs. 3 for ERC-20 (where the
/// amount is unindexed data instead).
fn transfer_topic() -> B256 {
    topic0("Transfer(address,address,uint256)")
}

fn approval_topic() -> B256 {
    topic0("Approval(address,address,uint256)")
}

fn transfer_single_topic() -> B256 {
    topic0("TransferSingle(address,address,address,uint256,uint256)")
}

fn transfer_batch_topic() -> B256 {
    topic0("TransferBatch(address,address,address,uint256[],uint256[])")
}

// ── Output types ──────────────────────────────────────────────────────────────

/// One decoded action found in a transaction. A single transaction can
/// contain several — e.g. a DEX swap emits multiple ERC-20 `Transfer` logs —
/// hence `NormalisedTransaction::decoded_actions` is a `Vec`, not an
/// `Option`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecodedAction {
    /// Plain ETH-style value transfer carried by the transaction itself
    /// (non-zero `value`, regardless of whether `to` is a contract).
    NativeTransfer {
        from: String,
        to: Option<String>,
        /// Wei, as a decimal string (U256 doesn't fit in JSON numbers).
        value: String,
    },

    Erc20Transfer {
        contract: String,
        from: String,
        to: String,
        /// Raw token units, decimal string (U256).
        value: String,
    },

    Erc20Approval {
        contract: String,
        owner: String,
        spender: String,
        value: String,
    },

    Erc721Transfer {
        contract: String,
        from: String,
        to: String,
        token_id: String,
    },

    Erc1155TransferSingle {
        contract: String,
        operator: String,
        from: String,
        to: String,
        token_id: String,
        value: String,
    },

    Erc1155TransferBatch {
        contract: String,
        operator: String,
        from: String,
        to: String,
        token_ids: Vec<String>,
        values: Vec<String>,
    },

    /// A decoded contract function call, produced only when the caller
    /// supplied a matching ABI via `decode_transaction`'s `abi` parameter.
    ContractCall {
        contract: String,
        function: String,
        /// Each argument rendered as `(name, value)`; values use Solidity
        /// display formatting (addresses as 0x-hex, arrays bracketed, etc.).
        args: Vec<(String, String)>,
    },

    /// A log or the calldata itself that we saw but could not decode —
    /// either because it doesn't match any known standard, or no ABI was
    /// supplied that recognises the function selector. Kept rather than
    /// dropped so callers can see that *something* happened here.
    Unknown {
        /// "log" or "calldata"
        source: &'static str,
        contract: Option<String>,
        /// First topic / 4-byte selector, if any, as 0x-hex.
        selector: Option<String>,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Decodes one transaction's native value transfer, calldata, and logs into
/// a flat list of `DecodedAction`s. `raw` must be the JSON produced by
/// `evm_rpc::get_transaction_data` (`{ transaction, receipt, timestamp }`).
///
/// `abi`, if supplied, is used to decode the top-level calldata (`input`)
/// against `transaction.to` when the calldata's function selector matches
/// something in the ABI. Standard ERC-20/721/1155 log decoding does not
/// require an ABI — those topics are recognised structurally.
///
/// This never returns `Err` for "couldn't decode" — only for genuinely
/// malformed input (e.g. `raw` missing the `transaction`/`receipt` keys
/// entirely). Anything it can't interpret becomes `DecodedAction::Unknown`.
pub fn decode_transaction(
    raw: &Value,
    abi: Option<&JsonAbi>,
) -> Result<Vec<DecodedAction>, AppError> {
    let transaction = raw
        .get("transaction")
        .ok_or_else(|| AppError::Normalisation("missing transaction".into()))?;
    let receipt = raw
        .get("receipt")
        .ok_or_else(|| AppError::Normalisation("missing receipt".into()))?;

    let mut actions = Vec::new();

    decode_native_value(transaction, &mut actions);
    decode_calldata(transaction, abi, &mut actions);
    decode_logs(receipt, &mut actions);

    Ok(actions)
}

// ── Native value transfer ──────────────────────────────────────────────────────

fn decode_native_value(transaction: &Value, actions: &mut Vec<DecodedAction>) {
    let Some(value) = transaction.get("value").and_then(Value::as_str) else {
        return;
    };
    let Ok(value) = U256::from_str(value) else {
        return;
    };
    if value.is_zero() {
        return;
    }

    let from = transaction
        .get("from")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let to = transaction
        .get("to")
        .and_then(Value::as_str)
        .map(String::from);

    actions.push(DecodedAction::NativeTransfer {
        from,
        to,
        value: value.to_string(),
    });
}

// ── Calldata (arbitrary contract call, only with a supplied ABI) ──────────────

fn decode_calldata(transaction: &Value, abi: Option<&JsonAbi>, actions: &mut Vec<DecodedAction>) {
    let Some(input) = transaction.get("input").and_then(Value::as_str) else {
        return;
    };
    let Ok(input_bytes) = hex::decode(input.trim_start_matches("0x")) else {
        return;
    };
    // Fewer than 4 bytes: no function selector, e.g. a plain value transfer
    // with empty calldata. Nothing to decode.
    if input_bytes.len() < 4 {
        return;
    }

    let contract = transaction
        .get("to")
        .and_then(Value::as_str)
        .map(String::from);
    let selector = format!("0x{}", hex::encode(&input_bytes[0..4]));

    let Some(abi) = abi else {
        actions.push(DecodedAction::Unknown {
            source: "calldata",
            contract,
            selector: Some(selector),
        });
        return;
    };

    let matched_function = abi
        .functions()
        .find(|f| f.selector().as_slice() == &input_bytes[0..4]);

    let Some(function) = matched_function else {
        actions.push(DecodedAction::Unknown {
            source: "calldata",
            contract,
            selector: Some(selector),
        });
        return;
    };

    match function.abi_decode_input(&input_bytes[4..]) {
        Ok(decoded_values) => {
            let args = function
                .inputs
                .iter()
                .zip(decoded_values.iter())
                .map(|(param, value)| (param.name.clone(), format_sol_value(value)))
                .collect();

            actions.push(DecodedAction::ContractCall {
                contract: contract.unwrap_or_default(),
                function: function.signature(),
                args,
            });
        }
        Err(_) => {
            // Selector matched but decode failed (mismatched ABI version,
            // truncated calldata, etc.) — still don't fail the whole tx.
            actions.push(DecodedAction::Unknown {
                source: "calldata",
                contract,
                selector: Some(selector),
            });
        }
    }
}

fn format_sol_value(value: &DynSolValue) -> String {
    match value {
        DynSolValue::Address(a) => a.to_string(),
        DynSolValue::Bool(b) => b.to_string(),
        DynSolValue::Bytes(b) => format!("0x{}", hex::encode(b)),
        DynSolValue::FixedBytes(b, _) => format!("0x{}", hex::encode(b.as_slice())),
        DynSolValue::String(s) => s.clone(),
        DynSolValue::Uint(v, _) => v.to_string(),
        DynSolValue::Int(v, _) => v.to_string(),
        DynSolValue::Array(items) | DynSolValue::FixedArray(items) => {
            let rendered: Vec<String> = items.iter().map(format_sol_value).collect();
            format!("[{}]", rendered.join(","))
        }
        DynSolValue::Tuple(items) => {
            let rendered: Vec<String> = items.iter().map(format_sol_value).collect();
            format!("({})", rendered.join(","))
        }
        other => format!("{other:?}"),
    }
}

// ── Logs (ERC-20/721/1155 standard events) ─────────────────────────────────────

fn decode_logs(receipt: &Value, actions: &mut Vec<DecodedAction>) {
    let Some(logs) = receipt.get("logs").and_then(Value::as_array) else {
        return;
    };

    for log in logs {
        decode_one_log(log, actions);
    }
}

fn decode_one_log(log: &Value, actions: &mut Vec<DecodedAction>) {
    let contract = log.get("address").and_then(Value::as_str).map(String::from);

    let Some(topics) = log.get("topics").and_then(Value::as_array) else {
        return;
    };
    let topic_strs: Vec<&str> = topics.iter().filter_map(Value::as_str).collect();
    let Some(first_topic) = topic_strs.first() else {
        return;
    };
    let Ok(topic0_bytes) = B256::from_str(first_topic) else {
        return;
    };

    let data = log
        .get("data")
        .and_then(Value::as_str)
        .and_then(|d| hex::decode(d.trim_start_matches("0x")).ok())
        .unwrap_or_default();

    let decoded = if topic0_bytes == transfer_topic() {
        decode_transfer_log(contract.as_deref(), &topic_strs, &data)
    } else if topic0_bytes == approval_topic() {
        decode_approval_log(contract.as_deref(), &topic_strs, &data)
    } else if topic0_bytes == transfer_single_topic() {
        decode_transfer_single_log(contract.as_deref(), &topic_strs, &data)
    } else if topic0_bytes == transfer_batch_topic() {
        decode_transfer_batch_log(contract.as_deref(), &topic_strs, &data)
    } else {
        None
    };

    match decoded {
        Some(action) => actions.push(action),
        None => actions.push(DecodedAction::Unknown {
            source: "log",
            contract,
            selector: Some(first_topic.to_string()),
        }),
    }
}

/// `Transfer(address indexed from, address indexed to, uint256 tokenId_or_value)`
/// ERC-20: value is unindexed (in `data`) → 3 topics total.
/// ERC-721: tokenId is indexed (topics[3]) → 4 topics total.
fn decode_transfer_log(
    contract: Option<&str>,
    topics: &[&str],
    data: &[u8],
) -> Option<DecodedAction> {
    let from = topic_to_address(topics.get(1)?)?;
    let to = topic_to_address(topics.get(2)?)?;
    let contract = contract?.to_string();

    if topics.len() >= 4 {
        // ERC-721: tokenId indexed as topics[3]
        let token_id = topic_to_u256(topics.get(3)?)?;
        Some(DecodedAction::Erc721Transfer {
            contract,
            from,
            to,
            token_id: token_id.to_string(),
        })
    } else {
        // ERC-20: value in data
        let value = bytes_to_u256(data)?;
        Some(DecodedAction::Erc20Transfer {
            contract,
            from,
            to,
            value: value.to_string(),
        })
    }
}

/// `Approval(address indexed owner, address indexed spender, uint256 value)`
fn decode_approval_log(
    contract: Option<&str>,
    topics: &[&str],
    data: &[u8],
) -> Option<DecodedAction> {
    let owner = topic_to_address(topics.get(1)?)?;
    let spender = topic_to_address(topics.get(2)?)?;
    let value = bytes_to_u256(data)?;

    Some(DecodedAction::Erc20Approval {
        contract: contract?.to_string(),
        owner,
        spender,
        value: value.to_string(),
    })
}

/// `TransferSingle(address indexed operator, address indexed from, address indexed to, uint256 id, uint256 value)`
fn decode_transfer_single_log(
    contract: Option<&str>,
    topics: &[&str],
    data: &[u8],
) -> Option<DecodedAction> {
    let operator = topic_to_address(topics.get(1)?)?;
    let from = topic_to_address(topics.get(2)?)?;
    let to = topic_to_address(topics.get(3)?)?;

    if data.len() < 64 {
        return None;
    }
    let token_id = bytes_to_u256(&data[0..32])?;
    let value = bytes_to_u256(&data[32..64])?;

    Some(DecodedAction::Erc1155TransferSingle {
        contract: contract?.to_string(),
        operator,
        from,
        to,
        token_id: token_id.to_string(),
        value: value.to_string(),
    })
}

/// `TransferBatch(address indexed operator, address indexed from, address indexed to, uint256[] ids, uint256[] values)`
/// Both arrays are dynamic types in unindexed `data`; decode via alloy-dyn-abi
/// rather than hand-rolling the offset/length arithmetic.
fn decode_transfer_batch_log(
    contract: Option<&str>,
    topics: &[&str],
    data: &[u8],
) -> Option<DecodedAction> {
    let operator = topic_to_address(topics.get(1)?)?;
    let from = topic_to_address(topics.get(2)?)?;
    let to = topic_to_address(topics.get(3)?)?;

    let event = alloy_json_abi::Event::parse(
        "TransferBatch(address indexed operator, address indexed from, address indexed to, uint256[] ids, uint256[] values)",
    )
    .ok()?;

    let topic_bytes: Vec<B256> = topics
        .iter()
        .filter_map(|t| B256::from_str(t).ok())
        .collect();
    let decoded = event.decode_log_parts(topic_bytes, data).ok()?;

    let mut ids = Vec::new();
    let mut values = Vec::new();
    for body_value in decoded.body {
        if let DynSolValue::Array(items) = body_value {
            let rendered: Vec<String> = items
                .iter()
                .filter_map(|v| match v {
                    DynSolValue::Uint(u, _) => Some(u.to_string()),
                    _ => None,
                })
                .collect();
            if ids.is_empty() {
                ids = rendered;
            } else {
                values = rendered;
            }
        }
    }

    Some(DecodedAction::Erc1155TransferBatch {
        contract: contract?.to_string(),
        operator,
        from,
        to,
        token_ids: ids,
        values,
    })
}

// ── Small helpers ─────────────────────────────────────────────────────────────

/// An indexed `address` parameter is topic-encoded as a full 32-byte word
/// with the address right-aligned in the last 20 bytes.
fn topic_to_address(topic: &str) -> Option<String> {
    let bytes = hex::decode(topic.trim_start_matches("0x")).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(Address::from_slice(&bytes[12..32]).to_string())
}

fn topic_to_u256(topic: &str) -> Option<U256> {
    let bytes = hex::decode(topic.trim_start_matches("0x")).ok()?;
    bytes_to_u256(&bytes)
}

fn bytes_to_u256(bytes: &[u8]) -> Option<U256> {
    if bytes.len() < 32 {
        return None;
    }
    Some(U256::from_be_slice(&bytes[0..32]))
}
