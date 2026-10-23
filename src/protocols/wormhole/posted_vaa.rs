//! The core bridge's posted VAA account, parsed in-house.
//!
//! A redeem on Solana (`complete_*`) carries no data: the message it redeems is the
//! VAA the guardians signed, which was posted to an account owned by the core bridge.
//! Layout (`PostedVAAData` in wormhole-foundation/wormhole, `solana/bridge`), all
//! integers little endian:
//!
//! ```text
//! 0   3   magic "vaa"
//! 3   1   VAA version
//! 4   1   consistency level
//! 5   4   VAA timestamp
//! 9   32  signature set account
//! 41  4   submission time
//! 45  4   nonce
//! 49  8   sequence
//! 57  2   emitter chain
//! 59  32  emitter address
//! 91  4   payload length
//! 95  n   payload
//! ```

const MAGIC: &[u8; 3] = b"vaa";
const PAYLOAD_LEN_AT: usize = 91;
const HEADER: usize = PAYLOAD_LEN_AT + 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostedVaa<'a> {
    pub sequence: u64,
    pub emitter_chain: u16,
    pub emitter_address: [u8; 32],
    pub payload: &'a [u8],
}

/// `None` when the bytes are not a posted VAA. Never panics.
pub fn parse(data: &[u8]) -> Option<PostedVaa<'_>> {
    if data.get(..3)? != MAGIC {
        return None;
    }
    let sequence = u64::from_le_bytes(data.get(49..57)?.try_into().ok()?);
    let emitter_chain = u16::from_le_bytes(data.get(57..59)?.try_into().ok()?);
    let emitter_address: [u8; 32] = data.get(59..91)?.try_into().ok()?;
    let len = u32::from_le_bytes(data.get(PAYLOAD_LEN_AT..HEADER)?.try_into().ok()?) as usize;
    let end = HEADER.checked_add(len)?;
    // The account is allocated to the exact size of the message, so anything
    // other than an exact fit is not a posted VAA this layout describes.
    if data.len() != end {
        return None;
    }
    Some(PostedVaa {
        sequence,
        emitter_chain,
        emitter_address,
        payload: data.get(HEADER..end)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(sequence: u64, chain: u16, emitter: [u8; 32], payload: &[u8]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(MAGIC);
        d.push(1); // version
        d.push(1); // consistency level
        d.extend_from_slice(&1u32.to_le_bytes()); // vaa time
        d.extend_from_slice(&[7; 32]); // signature set
        d.extend_from_slice(&2u32.to_le_bytes()); // submission time
        d.extend_from_slice(&3u32.to_le_bytes()); // nonce
        d.extend_from_slice(&sequence.to_le_bytes());
        d.extend_from_slice(&chain.to_le_bytes());
        d.extend_from_slice(&emitter);
        d.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        d.extend_from_slice(payload);
        d
    }

    #[test]
    fn parses_identity_and_payload() {
        let bytes = build(697_687, 2, [0xAA; 32], b"hello");
        let v = parse(&bytes).unwrap();
        assert_eq!(v.sequence, 697_687);
        assert_eq!(v.emitter_chain, 2);
        assert_eq!(v.emitter_address, [0xAA; 32]);
        assert_eq!(v.payload, b"hello");
    }

    #[test]
    fn rejects_the_wrong_magic() {
        let mut bytes = build(1, 2, [0; 32], b"x");
        bytes[0] = b'V';
        assert_eq!(parse(&bytes), None);
    }

    #[test]
    fn rejects_truncated_and_oversized_accounts() {
        let bytes = build(1, 2, [0; 32], b"payload");
        for cut in [0, 2, 3, 48, 94, bytes.len() - 1] {
            assert_eq!(parse(&bytes[..cut]), None, "cut at {cut}");
        }
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(parse(&long), None);
    }

    #[test]
    fn a_lying_payload_length_does_not_panic() {
        let mut bytes = build(1, 2, [0; 32], b"x");
        bytes[PAYLOAD_LEN_AT..HEADER].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse(&bytes), None);
    }
}
