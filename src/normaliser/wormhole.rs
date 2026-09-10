use serde_json::Value;

use crate::domain::bridge::BridgeEvent;
use crate::error::AppError;

pub fn normalise(raw: Value) -> Result<BridgeEvent, AppError> {
    let sequence = raw
        .get("sequence")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| AppError::Normalisation("missing sequence".into()))?;

    let emitter_chain = raw
        .get("emitterChain")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::Normalisation("missing emitterChain".into()))?;

    let emitter_address = raw
        .get("emitterAddress")
        .and_then(|e| e.get("hex"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let props = raw
        .get("content")
        .and_then(|c| c.get("standarizedProperties")); // note: their spelling

    let target_chain = props
        .and_then(|p| p.get("toChain"))
        .and_then(Value::as_u64)
        .filter(|&c| c != 0)
        .map(|c| c.to_string());

    let amount = props
        .and_then(|p| p.get("amount"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from);

    Ok(BridgeEvent {
        sequence,
        emitter_chain: emitter_chain.to_string(),
        emitter_address,
        target_chain,
        amount,
    })
}

pub fn extract_destination(_raw: &Value) -> Option<(u64, String)> {
    None
}
