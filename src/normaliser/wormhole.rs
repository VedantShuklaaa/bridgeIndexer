use serde_json::Value;

use crate::domain::bridge::BridgeEvent;
use crate::error::AppError;

pub fn normalise(raw: Value) -> Result<BridgeEvent, AppError> {
    let data = raw.get("data").unwrap_or(&raw);

    let sequence = data
        .get("sequence")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| data.get("sequence").and_then(Value::as_u64))
        .ok_or_else(|| AppError::Normalisation("missing sequence".into()))?;

    let emitter_chain = data
        .get("emitterChain")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let emitter_address = data
        .get("emitterAddr")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    Ok(BridgeEvent {
        sequence,
        emitter_chain,
        emitter_address,
        target_chain: data
            .get("toChain")
            .and_then(Value::as_str)
            .map(String::from),
        amount: data.get("amount").and_then(Value::as_str).map(String::from),
    })
}
