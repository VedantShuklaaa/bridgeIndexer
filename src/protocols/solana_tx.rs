//! A pure view of a Solana transaction, built from a Helius `getTransaction`
//! result (`jsonParsed` encoding). Adapters read this, never raw JSON.

use crate::error::AppError;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub program_id: String,
    /// Account pubkeys in instruction order. Empty for instructions the RPC parsed itself.
    pub accounts: Vec<String>,
    /// Raw instruction data. Empty for instructions the RPC parsed itself.
    pub data: Vec<u8>,
    /// The RPC's own parse (SPL token transfers and similar), when it has one.
    pub parsed: Option<Value>,
    /// Index of the top-level instruction this one ran under, or `None` if top-level.
    pub inner_of: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolanaTx {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    /// True when the transaction failed on chain (`meta.err` is set). A failed tx
    /// reverts its effects, so adapters must never index it as a transfer.
    pub failed: bool,
    /// Fee payer and first signer.
    pub signer: Option<String>,
    pub account_keys: Vec<String>,
    /// Top-level instructions first, then inner instructions, each tagged with `inner_of`.
    pub instructions: Vec<Instruction>,
    pub logs: Vec<String>,
}

impl SolanaTx {
    pub fn from_helius(raw: &Value, signature: &str) -> Result<Self, AppError> {
        let slot = raw
            .get("slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::Normalisation("missing slot".into()))?;
        let message = raw
            .pointer("/transaction/message")
            .ok_or_else(|| AppError::Normalisation("missing transaction.message".into()))?;

        let mut account_keys: Vec<String> = message
            .get("accountKeys")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::Normalisation("missing accountKeys".into()))?
            .iter()
            .filter_map(|k| {
                k.get("pubkey")
                    .and_then(Value::as_str)
                    .or_else(|| k.as_str())
                    .map(String::from)
            })
            .collect();

        // v0 transactions: addresses loaded from lookup tables. jsonParsed normally
        // folds these into accountKeys already, so only add what is missing.
        for field in ["writable", "readonly"] {
            if let Some(list) = raw
                .pointer(&format!("/meta/loadedAddresses/{field}"))
                .and_then(Value::as_array)
            {
                for k in list.iter().filter_map(Value::as_str) {
                    if !account_keys.iter().any(|a| a == k) {
                        account_keys.push(k.to_string());
                    }
                }
            }
        }

        let signer = account_keys.first().cloned();

        let mut instructions = Vec::new();
        if let Some(top) = message.get("instructions").and_then(Value::as_array) {
            for ix in top {
                instructions.push(parse_instruction(ix, None)?);
            }
        }
        if let Some(inner) = raw
            .pointer("/meta/innerInstructions")
            .and_then(Value::as_array)
        {
            for group in inner {
                let parent = group
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|i| i as usize);
                if let Some(list) = group.get("instructions").and_then(Value::as_array) {
                    for ix in list {
                        instructions.push(parse_instruction(ix, parent)?);
                    }
                }
            }
        }

        let logs = raw
            .pointer("/meta/logMessages")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        Ok(Self {
            signature: signature.to_string(),
            slot,
            block_time: raw.get("blockTime").and_then(Value::as_i64),
            failed: raw.pointer("/meta/err").is_some_and(|e| !e.is_null()),
            signer,
            account_keys,
            instructions,
            logs,
        })
    }

    /// Every program invoked anywhere in the tx, top-level or inner.
    pub fn program_ids(&self) -> HashSet<&str> {
        self.instructions
            .iter()
            .map(|i| i.program_id.as_str())
            .collect()
    }

    pub fn instructions_of<'a>(
        &'a self,
        program_id: &'a str,
    ) -> impl Iterator<Item = &'a Instruction> {
        self.instructions
            .iter()
            .filter(move |i| i.program_id == program_id)
    }
}

fn parse_instruction(ix: &Value, inner_of: Option<usize>) -> Result<Instruction, AppError> {
    let program_id = ix
        .get("programId")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Normalisation("instruction without programId".into()))?
        .to_string();

    let accounts = ix
        .get("accounts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    // Instructions the RPC cannot parse carry base58 `data`. A bad string is a
    // malformed instruction, not a reason to drop the whole transaction.
    let data = match ix.get("data").and_then(Value::as_str) {
        Some(s) => bs58::decode(s).into_vec().unwrap_or_default(),
        None => Vec::new(),
    };

    Ok(Instruction {
        program_id,
        accounts,
        data,
        parsed: ix.get("parsed").cloned(),
        inner_of,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "slot": 42,
            "blockTime": 1_700_000_000,
            "meta": {
                "err": null,
                "logMessages": ["Program A invoke [1]"],
                "loadedAddresses": { "writable": ["LutWritable"], "readonly": ["LutReadonly"] },
                "innerInstructions": [{
                    "index": 0,
                    "instructions": [{ "programId": "Inner", "accounts": ["x"], "data": "3Bxs" }]
                }]
            },
            "transaction": {
                "message": {
                    "accountKeys": [
                        { "pubkey": "Signer", "signer": true },
                        { "pubkey": "LutWritable", "signer": false }
                    ],
                    "instructions": [
                        { "programId": "Outer", "accounts": ["Signer", "LutReadonly"], "data": "3Bxs" },
                        { "program": "spl-token", "programId": "Tok", "parsed": { "type": "transfer" } }
                    ]
                }
            }
        })
    }

    #[test]
    fn builds_the_view_from_jsonparsed() {
        let tx = SolanaTx::from_helius(&sample(), "sig").unwrap();
        assert_eq!(tx.signature, "sig");
        assert_eq!(tx.slot, 42);
        assert_eq!(tx.signer.as_deref(), Some("Signer"));
        assert_eq!(tx.logs.len(), 1);
        assert_eq!(tx.instructions.len(), 3);
    }

    #[test]
    fn loaded_addresses_are_added_once() {
        let tx = SolanaTx::from_helius(&sample(), "sig").unwrap();
        assert_eq!(
            tx.account_keys,
            vec!["Signer", "LutWritable", "LutReadonly"]
        );
    }

    #[test]
    fn inner_instructions_are_tagged_with_their_parent() {
        let tx = SolanaTx::from_helius(&sample(), "sig").unwrap();
        let inner = tx.instructions_of("Inner").next().unwrap();
        assert_eq!(inner.inner_of, Some(0));
        assert_eq!(tx.instructions_of("Outer").next().unwrap().inner_of, None);
    }

    #[test]
    fn program_ids_include_inner_calls() {
        let tx = SolanaTx::from_helius(&sample(), "sig").unwrap();
        let ids = tx.program_ids();
        assert!(ids.contains("Outer") && ids.contains("Inner") && ids.contains("Tok"));
    }

    #[test]
    fn data_is_base58_decoded_and_parsed_is_kept() {
        let tx = SolanaTx::from_helius(&sample(), "sig").unwrap();
        let outer = tx.instructions_of("Outer").next().unwrap();
        assert_eq!(outer.data, bs58::decode("3Bxs").into_vec().unwrap());
        let tok = tx.instructions_of("Tok").next().unwrap();
        assert!(tok.data.is_empty() && tok.parsed.is_some());
    }

    #[test]
    fn failed_follows_meta_err() {
        let ok = SolanaTx::from_helius(&sample(), "sig").unwrap();
        assert!(!ok.failed);

        let mut raw = sample();
        raw["meta"]["err"] = json!({ "InstructionError": [0, "Custom"] });
        assert!(SolanaTx::from_helius(&raw, "sig").unwrap().failed);
    }

    #[test]
    fn missing_pieces_are_normalisation_errors_not_panics() {
        assert!(SolanaTx::from_helius(&json!({}), "s").is_err());
        assert!(SolanaTx::from_helius(&json!({"slot": 1}), "s").is_err());
        assert!(
            SolanaTx::from_helius(&json!({"slot": 1, "transaction": {"message": {}}}), "s")
                .is_err()
        );
    }
}
