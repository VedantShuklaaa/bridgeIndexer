//! Pure decode tests: no network, no Docker.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bridge::error::AppError;
use bridge::vaa::decode::{
    decode_destination_address, decode_vaa, format_amount, to_evm_address, to_solana_address,
};

/// Builds a VAA with `num_sigs` dummy signatures, then body, then payload.
fn build_vaa(
    num_sigs: u8,
    emitter_chain: u16,
    emitter: [u8; 32],
    sequence: u64,
    payload: &[u8],
) -> String {
    let mut b = Vec::new();
    b.push(1); // version
    b.extend_from_slice(&0u32.to_be_bytes()); // guardian set index
    b.push(num_sigs);
    for _ in 0..num_sigs {
        b.extend_from_slice(&[0xEE; 66]); // index + 65-byte signature
    }
    b.extend_from_slice(&0u32.to_be_bytes()); // timestamp
    b.extend_from_slice(&0u32.to_be_bytes()); // nonce
    b.extend_from_slice(&emitter_chain.to_be_bytes());
    b.extend_from_slice(&emitter);
    b.extend_from_slice(&sequence.to_be_bytes());
    b.push(1); // consistency level
    b.extend_from_slice(payload);
    STANDARD.encode(b)
}

fn transfer_payload(
    payload_id: u8,
    amount: u128,
    token: [u8; 32],
    token_chain: u16,
    to: [u8; 32],
    to_chain: u16,
) -> Vec<u8> {
    let mut p = vec![payload_id];
    let mut amt = [0u8; 32];
    amt[16..].copy_from_slice(&amount.to_be_bytes());
    p.extend_from_slice(&amt);
    p.extend_from_slice(&token);
    p.extend_from_slice(&token_chain.to_be_bytes());
    p.extend_from_slice(&to);
    p.extend_from_slice(&to_chain.to_be_bytes());
    p.extend_from_slice(&[0u8; 32]); // fee (type 1) / from address (type 3), ignored
    p
}

#[test]
fn decodes_type_1_transfer() {
    let payload = transfer_payload(1, 1_500_000, [0x11; 32], 2, [0x22; 32], 5);
    let vaa = build_vaa(0, 1, [0xAB; 32], 42, &payload);

    let d = decode_vaa(&vaa).unwrap();

    assert_eq!(d.header.emitter_chain, 1);
    assert_eq!(d.header.emitter_address, hex::encode([0xAB; 32]));
    assert_eq!(d.header.sequence, 42);

    let t = d
        .transfer
        .expect("type 1 payload should decode as a transfer");
    assert_eq!(t.amount, 1_500_000);
    assert_eq!(t.token_address, hex::encode([0x11; 32]));
    assert_eq!(t.token_chain, 2);
    assert_eq!(t.to_address, hex::encode([0x22; 32]));
    assert_eq!(t.to_chain, 5);
}

#[test]
fn decodes_type_3_transfer_with_payload() {
    let payload = transfer_payload(3, 7, [0x33; 32], 1, [0x44; 32], 2);
    let vaa = build_vaa(0, 1, [0xCD; 32], 9, &payload);

    let t = decode_vaa(&vaa)
        .unwrap()
        .transfer
        .expect("type 3 is a transfer");
    assert_eq!(t.amount, 7);
    assert_eq!(t.token_chain, 1);
    assert_eq!(t.to_chain, 2);
}

#[test]
fn non_transfer_payload_has_no_transfer() {
    // payload id 2 is an asset attestation, not a transfer
    let vaa = build_vaa(0, 1, [1; 32], 1, &[2, 0, 0, 0]);
    assert!(decode_vaa(&vaa).unwrap().transfer.is_none());
}

#[test]
fn empty_payload_has_no_transfer() {
    let vaa = build_vaa(0, 1, [1; 32], 1, &[]);
    assert!(decode_vaa(&vaa).unwrap().transfer.is_none());
}

#[test]
fn signature_count_shifts_the_body() {
    let payload = transfer_payload(1, 99, [5; 32], 1, [6; 32], 2);
    let vaa = build_vaa(13, 1, [0xAB; 32], 777, &payload);

    let d = decode_vaa(&vaa).unwrap();
    assert_eq!(d.header.sequence, 777);
    assert_eq!(d.transfer.unwrap().amount, 99);
}

#[test]
fn amount_uses_full_u128_range() {
    let amount = u128::MAX / 3;
    let payload = transfer_payload(1, amount, [0; 32], 1, [0; 32], 2);
    let vaa = build_vaa(0, 1, [0; 32], 1, &payload);
    assert_eq!(decode_vaa(&vaa).unwrap().transfer.unwrap().amount, amount);
}

#[test]
fn rejects_invalid_base64() {
    assert!(matches!(
        decode_vaa("not base64 !!!"),
        Err(AppError::Normalisation(_))
    ));
}

#[test]
fn rejects_too_short_for_header() {
    let vaa = STANDARD.encode([1u8, 0, 0, 0, 0]);
    assert!(matches!(decode_vaa(&vaa), Err(AppError::Normalisation(_))));
}

#[test]
fn rejects_truncated_body() {
    let mut bytes = vec![1u8, 0, 0, 0, 0, 0]; // header, zero signatures
    bytes.extend_from_slice(&[0u8; 20]); // body shorter than the 51 fixed bytes
    assert!(matches!(
        decode_vaa(&STANDARD.encode(bytes)),
        Err(AppError::Normalisation(_))
    ));
}

#[test]
fn rejects_transfer_payload_that_is_too_short() {
    let mut payload = vec![1u8];
    payload.extend_from_slice(&[0u8; 49]); // 50 bytes, needs 101
    let vaa = build_vaa(0, 1, [1; 32], 1, &payload);
    assert!(matches!(decode_vaa(&vaa), Err(AppError::Normalisation(_))));
}

// ---- address decoding ----

fn word(last20: &str) -> String {
    format!("{}{}", "0".repeat(24), last20)
}

#[test]
fn evm_address_takes_the_last_20_bytes() {
    let hex32 = word("d78d199f8c402e7b5cc2abe278df0412400a3bae");
    assert_eq!(
        to_evm_address(&hex32).as_deref(),
        Some("0xd78d199f8c402e7b5cc2abe278df0412400a3bae")
    );
}

#[test]
fn evm_address_rejects_bad_input() {
    assert_eq!(to_evm_address("zz"), None);
    assert_eq!(to_evm_address("00"), None); // valid hex, wrong length
}

#[test]
fn solana_address_is_base58_of_the_32_bytes() {
    let hex32 = hex::encode([0u8; 32]);
    assert_eq!(
        to_solana_address(&hex32).as_deref(),
        Some("11111111111111111111111111111111")
    );
    assert_eq!(to_solana_address("00"), None);
}

#[test]
fn destination_address_dispatches_on_chain() {
    let hex32 = word("d78d199f8c402e7b5cc2abe278df0412400a3bae");
    // EVM chains
    for chain in [2u16, 4, 5, 6, 23, 24, 25, 30] {
        assert_eq!(
            decode_destination_address(chain, &hex32).as_deref(),
            Some("0xd78d199f8c402e7b5cc2abe278df0412400a3bae"),
            "chain {chain}"
        );
    }
    // Solana
    assert_eq!(
        decode_destination_address(1, &hex::encode([0u8; 32])).as_deref(),
        Some("11111111111111111111111111111111")
    );
    // anything else keeps the raw 32-byte word
    assert_eq!(
        decode_destination_address(15, &hex32),
        Some(format!("0x{hex32}"))
    );
}

// ---- amount formatting ----
// Wormhole normalises amounts to at most 8 decimals, so format_amount caps
// the divisor at 8 even for tokens with more decimals.

#[test]
fn format_amount_basic() {
    assert_eq!(format_amount(100_000_000, Some(8)), "1");
    assert_eq!(format_amount(150_000_000, Some(8)), "1.5");
    assert_eq!(format_amount(1_500_000, Some(6)), "1.5");
    assert_eq!(format_amount(1_000_000_000, Some(8)), "10");
}

#[test]
fn format_amount_zero_and_dust() {
    assert_eq!(format_amount(0, Some(6)), "0");
    assert_eq!(format_amount(1, Some(8)), "0.00000001");
    assert_eq!(format_amount(123, Some(2)), "1.23");
}

#[test]
fn format_amount_zero_decimals() {
    assert_eq!(format_amount(5, Some(0)), "5");
    assert_eq!(format_amount(0, Some(0)), "0");
}

#[test]
fn format_amount_caps_at_8_decimals() {
    // an 18-decimals token arrives already normalised to 8
    assert_eq!(format_amount(123, Some(18)), "0.00000123");
    assert_eq!(format_amount(100_000_000, Some(18)), "1");
}

#[test]
fn format_amount_unknown_decimals_assumes_8() {
    assert_eq!(format_amount(1_000_000, None), "0.01");
}

#[test]
fn format_amount_does_not_overflow_on_max() {
    // must not panic
    let s = format_amount(u128::MAX, Some(8));
    assert!(s.contains('.') || s.chars().all(|c| c.is_ascii_digit()));
}
