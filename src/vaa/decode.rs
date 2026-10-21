use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct VaaHeader {
    pub emitter_chain: u16,
    pub emitter_address: String,
    pub sequence: u64,
}

#[derive(Debug, Clone)]
pub struct TokenTransferPayload {
    pub amount: u128,
    pub token_address: String,
    pub token_chain: u16,
    pub to_address: String,
    pub to_chain: u16,
}

fn is_evm_chain(chain_id: u16) -> bool {
    matches!(
        chain_id,
        2 | 4 | 5 | 6 | 23 | 24 | 25 | 30 | 36 | 39 | 46 | 47
    )
}

pub fn decode_destination_address(chain_id: u16, hex32: &str) -> Option<String> {
    if is_evm_chain(chain_id) {
        to_evm_address(hex32)
    } else if chain_id == 1 {
        to_solana_address(hex32)
    } else {
        Some(format!("0x{hex32}"))
    }
}
pub struct DecodedVaa {
    pub header: VaaHeader,
    pub transfer: Option<TokenTransferPayload>,
}

pub fn decode_vaa(vaa_b64: &str) -> Result<DecodedVaa, AppError> {
    let bytes = STANDARD
        .decode(vaa_b64)
        .map_err(|e| AppError::Normalisation(format!("invalid VAA base64: {e}")))?;

    if bytes.len() < 6 {
        return Err(AppError::Normalisation("VAA too short for header".into()));
    }

    let num_signatures = bytes[5] as usize;
    let body_start = 6 + (num_signatures * 66);

    if bytes.len() < body_start + 51 {
        return Err(AppError::Normalisation("VAA body truncated".into()));
    }

    let body = &bytes[body_start..];

    let emitter_chain = u16::from_be_bytes(body[8..10].try_into().unwrap());
    let emitter_address = hex::encode(&body[10..42]);
    let sequence = u64::from_be_bytes(body[42..50].try_into().unwrap());

    // ---- paste goes here, replacing the two lines below ----
    let payload = &body[51..];
    tracing::debug!(payload_id = ?payload.first(), payload_len = payload.len(), "vaa payload");

    let transfer = match payload.first() {
        Some(1) | Some(3) => Some(decode_token_transfer(payload)?),
        _ => None,
    };
    // ---- end paste ----

    Ok(DecodedVaa {
        header: VaaHeader {
            emitter_chain,
            emitter_address,
            sequence,
        },
        transfer,
    })
}

fn decode_token_transfer(payload: &[u8]) -> Result<TokenTransferPayload, AppError> {
    if payload.len() < 101 {
        return Err(AppError::Normalisation("transfer payload too short".into()));
    }

    Ok(TokenTransferPayload {
        amount: u128_from_last16(&payload[1..33]),
        token_address: hex::encode(&payload[33..65]),
        token_chain: u16::from_be_bytes(payload[65..67].try_into().unwrap()),
        to_address: hex::encode(&payload[67..99]),
        to_chain: u16::from_be_bytes(payload[99..101].try_into().unwrap()),
    })
}

fn u128_from_last16(bytes32: &[u8]) -> u128 {
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes32[16..32]);
    u128::from_be_bytes(buf)
}

/// (last 20 bytes).
pub fn to_evm_address(hex32: &str) -> Option<String> {
    let bytes = hex::decode(hex32).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(format!("0x{}", hex::encode(&bytes[12..32])))
}

pub fn to_solana_address(hex32: &str) -> Option<String> {
    let bytes = hex::decode(hex32).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    Some(bs58::encode(bytes).into_string())
}

pub fn format_amount(raw: u128, real_decimals: Option<u8>) -> String {
    let decimals = real_decimals.unwrap_or(8).min(8) as u32;
    let divisor = 10u128.pow(decimals);
    let whole = raw / divisor;
    let frac = raw % divisor;

    if decimals == 0 {
        return whole.to_string();
    }
    format!("{whole}.{:0width$}", frac, width = decimals as usize)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
