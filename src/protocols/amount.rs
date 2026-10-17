use serde::{Deserialize, Serialize};
use std::fmt;

/// Largest value a `NUMERIC(78,0)` column holds that we need: a `u256` has at most 78 digits.
const MAX_DIGITS: usize = 78;

/// A non-negative integer amount in a protocol's raw units, kept as a decimal string.
///
/// Stored in `NUMERIC(78,0)`, so amounts up to `u256` fit. A string avoids a 256-bit
/// integer dependency, and the invariant (digits only, no leading zeros) is checked
/// once at construction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawAmount(String);

impl RawAmount {
    pub fn parse(s: &str) -> Option<Self> {
        let valid = !s.is_empty()
            && s.len() <= MAX_DIGITS
            && s.bytes().all(|b| b.is_ascii_digit())
            && (s == "0" || !s.starts_with('0'));
        valid.then(|| Self(s.to_string()))
    }

    pub fn from_u128(v: u128) -> Self {
        Self(v.to_string())
    }

    /// Big-endian bytes, at most 32 of them (a `u256`).
    pub fn from_be_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 32 {
            return None;
        }
        let mut n: Vec<u8> = bytes.to_vec();
        let mut digits: Vec<u8> = Vec::new();
        while n.iter().any(|b| *b != 0) {
            let mut rem = 0u16;
            for b in n.iter_mut() {
                let cur = (rem << 8) | u16::from(*b);
                *b = (cur / 10) as u8;
                rem = cur % 10;
            }
            digits.push(b'0' + rem as u8);
        }
        if digits.is_empty() {
            digits.push(b'0');
        }
        digits.reverse();
        Some(Self(String::from_utf8(digits).expect("ascii digits")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RawAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_plain_integers() {
        assert_eq!(RawAmount::parse("0").unwrap().as_str(), "0");
        assert_eq!(RawAmount::parse("1000000").unwrap().as_str(), "1000000");
    }

    #[test]
    fn parse_rejects_bad_input() {
        for bad in ["", "01", "-1", "1.5", "1e9", " 1", "abc"] {
            assert!(
                RawAmount::parse(bad).is_none(),
                "{bad:?} should be rejected"
            );
        }
        assert!(RawAmount::parse(&"9".repeat(79)).is_none());
        assert!(RawAmount::parse(&"9".repeat(78)).is_some());
    }

    #[test]
    fn from_be_bytes_matches_integer_math() {
        assert_eq!(RawAmount::from_be_bytes(&[]).unwrap().as_str(), "0");
        assert_eq!(RawAmount::from_be_bytes(&[0, 0]).unwrap().as_str(), "0");
        assert_eq!(
            RawAmount::from_be_bytes(&[0x01, 0x00]).unwrap().as_str(),
            "256"
        );
        let v: u128 = 123_456_789_012_345_678_901_234_567_890;
        assert_eq!(
            RawAmount::from_be_bytes(&v.to_be_bytes()).unwrap(),
            RawAmount::from_u128(v)
        );
    }

    #[test]
    fn from_be_bytes_covers_the_full_u256_range() {
        let max = RawAmount::from_be_bytes(&[0xff; 32]).unwrap();
        assert_eq!(
            max.as_str(),
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
        );
        assert!(RawAmount::from_be_bytes(&[0xff; 33]).is_none());
    }
}
