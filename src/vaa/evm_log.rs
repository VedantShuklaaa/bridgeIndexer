use serde_json::Value;
use sha3::{Digest, Keccak256};

use crate::error::AppError;

/// A Wormhole message as it appears on the *source* chain, before any
/// guardian ever signs it. Everything a guardian would put in the VAA
/// header is already sitting in this event — we just read it ourselves.
pub struct SourceMessage {
    pub emitter_chain: u16,
    /// 32-byte hex, no `0x` prefix — same format `vaa::decode` already uses
    /// for `VaaHeader::emitter_address`, so downstream code is unaffected.
    pub emitter_address: String,
    pub sequence: u64,
    pub nonce: u32,
    pub consistency_level: u8,
    pub payload: Vec<u8>,
}

fn log_message_published_topic0() -> String {
    let sig = b"LogMessagePublished(address,uint64,uint32,bytes,uint8)";
    format!("0x{}", hex::encode(Keccak256::digest(sig)))
}

/// Reads the Core Bridge's `LogMessagePublished` event straight out of an
/// `eth_getTransactionReceipt` result (the same receipt `clients::evm`
/// already fetched for the source transaction — no extra RPC call, and no
/// external indexer).
///
/// event LogMessagePublished(
///     address indexed sender,
///     uint64 sequence,
///     uint32 nonce,
///     bytes payload,
///     uint8 consistencyLevel
/// );
///
/// `sender` is indexed (topics[1]); `sequence`, `nonce`, `payload`, and
/// `consistencyLevel` are ABI-encoded together in `data` using standard
/// head/tail encoding for a dynamic type: four 32-byte head words
/// (sequence, nonce, offset-to-payload, consistencyLevel) followed by the
/// payload's own length-prefixed tail.
pub fn extract_source_message(
    receipt: &Value,
    core_bridge_contract: &str,
    own_wormhole_chain_id: u16,
) -> Result<SourceMessage, AppError> {
    let logs = receipt
        .get("logs")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Normalisation("receipt has no logs".into()))?;

    let topic0 = log_message_published_topic0();
    let core_bridge_lower = core_bridge_contract.to_lowercase();

    let log = logs
        .iter()
        .find(|log| {
            let address_matches = log
                .get("address")
                .and_then(Value::as_str)
                .map(|a| a.eq_ignore_ascii_case(&core_bridge_lower))
                .unwrap_or(false);

            let topic_matches = log
                .get("topics")
                .and_then(Value::as_array)
                .and_then(|t| t.first())
                .and_then(Value::as_str)
                .map(|t| t.eq_ignore_ascii_case(&topic0))
                .unwrap_or(false);

            address_matches && topic_matches
        })
        .ok_or_else(|| {
            AppError::Normalisation(
                "no LogMessagePublished event from the core bridge in this transaction".into(),
            )
        })?;

    let topics = log
        .get("topics")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::Normalisation("log missing topics".into()))?;

    let sender_topic = topics
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Normalisation("log missing sender topic".into()))?;
    let emitter_address = sender_topic.trim_start_matches("0x").to_lowercase();

    let data_hex = log
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Normalisation("log missing data".into()))?;
    let data = hex::decode(data_hex.trim_start_matches("0x"))
        .map_err(|e| AppError::Normalisation(format!("bad log data hex: {e}")))?;

    if data.len() < 128 {
        return Err(AppError::Normalisation(
            "LogMessagePublished data shorter than the fixed head".into(),
        ));
    }

    let sequence = u64::from_be_bytes(data[24..32].try_into().unwrap());
    let nonce = u32::from_be_bytes(data[60..64].try_into().unwrap());
    let payload_offset = u64::from_be_bytes(data[88..96].try_into().unwrap()) as usize;
    let consistency_level = data[127];

    if payload_offset + 32 > data.len() {
        return Err(AppError::Normalisation(
            "payload offset points outside log data".into(),
        ));
    }

    let payload_len = u64::from_be_bytes(
        data[payload_offset + 24..payload_offset + 32]
            .try_into()
            .unwrap(),
    ) as usize;
    let payload_start = payload_offset + 32;
    let payload_end = payload_start
        .checked_add(payload_len)
        .ok_or_else(|| AppError::Normalisation("payload length overflow".into()))?;

    if payload_end > data.len() {
        return Err(AppError::Normalisation(
            "payload length points outside log data".into(),
        ));
    }

    Ok(SourceMessage {
        emitter_chain: own_wormhole_chain_id,
        emitter_address,
        sequence,
        nonce,
        consistency_level,
        payload: data[payload_start..payload_end].to_vec(),
    })
}
