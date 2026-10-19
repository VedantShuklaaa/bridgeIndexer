//! Chain-first detection. The adapters decode a transfer from Solana data alone, this
//! module turns it into a [`BridgeTransfer`] with status `Detected`, and WormholeScan
//! data is layered on top afterwards. Enrichment can raise the status but never gates
//! persistence, and it is only applied when the VAA matches the message decoded on chain.

use crate::chains::{NS_WORMHOLE, RawChainRef};
use crate::domain::bridge_transfer::{BridgeMessageId, BridgeStatus, BridgeTransfer, ChainId};
use crate::error::AppError;
use crate::protocols::{
    Classification, DetectedTransfer, MessageId, SolanaTx, compose::compose,
    registry::ProtocolRegistry,
};
use crate::services::token_metadata::TokenMetadata;
use crate::util::explorer::explorer_tx_url;
use crate::vaa::decode::{decode_vaa, format_amount};
use serde_json::Value;

/// Runs every adapter that matches the tx and merges the result, one transfer per
/// message identity. Rejections are logged with their kind and reason.
pub fn detect(registry: &ProtocolRegistry, tx: &SolanaTx) -> Vec<DetectedTransfer> {
    let mut found = Vec::new();
    for (protocol, classification) in registry.classify(tx) {
        match classification {
            Classification::Transfers(transfers) => found.extend(transfers),
            Classification::Rejected(reject) => tracing::info!(
                protocol = protocol.as_str(),
                kind = reject.kind(),
                reason = reject.reason(),
                signature = %tx.signature,
                "not indexed as a transfer"
            ),
        }
    }
    compose(found)
}

/// The token's chain (Wormhole ID) and address, when the transfer carries a token.
pub fn token_lookup(d: &DetectedTransfer) -> Option<(u16, String)> {
    let token = d.token.as_ref()?;
    Some((wormhole_id(&token.chain).ok()?, token.address.clone()))
}

fn wormhole_id(chain: &RawChainRef) -> Result<u16, AppError> {
    if chain.protocol != NS_WORMHOLE {
        return Err(AppError::Normalisation(format!(
            "chain reference from {:?}, expected {NS_WORMHOLE}",
            chain.protocol
        )));
    }
    chain
        .id
        .parse()
        .map_err(|_| AppError::Normalisation(format!("bad wormhole chain id {:?}", chain.id)))
}

/// A decoded transfer as the API's `BridgeTransfer`. Status is `Detected`: the chain
/// shows the send, nothing else has confirmed it yet.
pub fn to_bridge_transfer(
    d: &DetectedTransfer,
    metadata: Option<TokenMetadata>,
) -> Result<BridgeTransfer, AppError> {
    let MessageId::Wormhole {
        emitter_chain,
        emitter_address,
        sequence,
    } = &d.message;

    let source_id = wormhole_id(&d.source)?;
    let destination_id = wormhole_id(&d.destination)?;

    let raw_amount: Option<u128> = d.amount_raw.as_ref().and_then(|a| a.as_str().parse().ok());
    let decimals = metadata.as_ref().and_then(|m| m.decimals);

    Ok(BridgeTransfer {
        source_chain: ChainId::from_wormhole_id(source_id),
        source_tx_hash: d.observed_tx.clone(),
        source_wallet: d.source_wallet.clone(),
        source_explorer_url: explorer_tx_url(source_id, &d.observed_tx),
        destination_chain: ChainId::from_wormhole_id(destination_id),
        destination_wallet: d.destination_wallet.clone(),
        destination_tx_hash: None,
        destination_explorer_url: None,
        token: d.token.as_ref().map(|t| t.address.clone()),
        token_symbol: metadata.and_then(|m| m.symbol),
        amount: d.amount_raw.as_ref().map(|a| a.to_string()),
        amount_formatted: raw_amount.map(|r| format_amount(r, decimals)),
        message_id: BridgeMessageId {
            emitter_chain: *emitter_chain,
            emitter_address: emitter_address.clone(),
            sequence: *sequence,
        },
        status: BridgeStatus::Detected,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enrichment {
    /// The VAA matches the decoded message: status raised, destination tx filled if known.
    Applied,
    /// WormholeScan has no usable VAA yet. The transfer stays `Detected`.
    NoVaa,
    /// The VAA describes a different message. Ignored, the chain wins.
    Mismatch,
}

/// Layers a WormholeScan `/operations` response onto a detected transfer.
///
/// `Pending` means WormholeScan has the VAA, `Completed` means it also knows the
/// destination tx. Anything that does not match the on-chain message changes nothing.
pub fn apply_wormholescan(transfer: &mut BridgeTransfer, wh_raw: &Value) -> Enrichment {
    let Some(vaa) = wh_raw
        .get("vaa")
        .and_then(|v| v.get("raw"))
        .and_then(Value::as_str)
    else {
        return Enrichment::NoVaa;
    };
    let Ok(decoded) = decode_vaa(vaa) else {
        return Enrichment::NoVaa;
    };

    let id = &transfer.message_id;
    if decoded.header.emitter_chain != id.emitter_chain
        || decoded.header.emitter_address != id.emitter_address
        || decoded.header.sequence != id.sequence
    {
        return Enrichment::Mismatch;
    }

    // Only raise: a transfer that is already `Pending` or `Completed` keeps its status.
    if matches!(transfer.status, BridgeStatus::Detected) {
        transfer.status = BridgeStatus::Pending;
    }

    if let Some(hash) = destination_tx_from_operation(wh_raw) {
        apply_destination_tx(transfer, &hash);
    }
    Enrichment::Applied
}

/// Destination tx inside a WormholeScan `/operations` item. Paths are tried in order;
/// the first non-empty string wins.
pub fn destination_tx_from_operation(raw: &Value) -> Option<String> {
    const PATHS: [&[&str]; 3] = [
        &["targetChain", "transaction", "txHash"],
        &["globalTx", "destinationTx", "txHash"],
        &["destinationTx", "txHash"],
    ];
    PATHS.iter().find_map(|path| {
        let mut cur = raw;
        for key in *path {
            cur = cur.get(*key)?;
        }
        cur.as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    })
}

/// Completes a `Pending` transfer once its destination tx is known. Never touches
/// `Detected` (no matched VAA) and never rewrites a `Completed` transfer.
pub fn apply_destination_tx(transfer: &mut BridgeTransfer, hash: &str) -> bool {
    let hash = hash.trim();
    if hash.is_empty() || !matches!(transfer.status, BridgeStatus::Pending) {
        return false;
    }
    transfer.destination_explorer_url =
        explorer_tx_url(transfer.destination_chain.wormhole_id(), hash);
    transfer.destination_tx_hash = Some(hash.to_string());
    transfer.status = BridgeStatus::Completed;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::{Direction, TokenRef, amount::RawAmount};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use serde_json::json;

    const EMITTER_HEX: &str = "0909090909090909090909090909090909090909090909090909090909090909";

    fn detected() -> DetectedTransfer {
        DetectedTransfer {
            message: MessageId::Wormhole {
                emitter_chain: 1,
                emitter_address: EMITTER_HEX.into(),
                sequence: 42,
            },
            direction: Direction::Outbound,
            observed_tx: "sig".into(),
            event_index: 0,
            source: RawChainRef::wormhole(1),
            destination: RawChainRef::wormhole(2),
            source_wallet: Some("Wallet".into()),
            destination_wallet: Some("0xabc".into()),
            token: Some(TokenRef {
                chain: RawChainRef::wormhole(1),
                address: "TokenMint".into(),
            }),
            amount_raw: RawAmount::parse("5000000"),
            via: None,
        }
    }

    /// A VAA with no signatures and an empty payload: header and the 51 byte body prefix.
    fn vaa_b64(chain: u16, emitter: [u8; 32], sequence: u64) -> String {
        let mut bytes = vec![1u8, 0, 0, 0, 0, 0]; // version, guardian set, 0 signatures
        bytes.extend_from_slice(&[0u8; 8]); // timestamp, nonce
        bytes.extend_from_slice(&chain.to_be_bytes());
        bytes.extend_from_slice(&emitter);
        bytes.extend_from_slice(&sequence.to_be_bytes());
        bytes.push(1); // consistency level
        STANDARD.encode(bytes)
    }

    fn transfer() -> BridgeTransfer {
        to_bridge_transfer(&detected(), None).unwrap()
    }

    #[test]
    fn a_detected_transfer_maps_to_the_api_shape_as_detected() {
        let t = to_bridge_transfer(
            &detected(),
            Some(TokenMetadata {
                symbol: Some("USDC".into()),
                decimals: Some(6),
            }),
        )
        .unwrap();

        assert!(matches!(t.status, BridgeStatus::Detected));
        assert_eq!(t.source_chain, ChainId::Solana);
        assert_eq!(t.destination_chain, ChainId::Ethereum);
        assert_eq!(t.source_tx_hash, "sig");
        assert_eq!(t.source_wallet.as_deref(), Some("Wallet"));
        assert_eq!(t.destination_wallet.as_deref(), Some("0xabc"));
        assert_eq!(t.token.as_deref(), Some("TokenMint"));
        assert_eq!(t.token_symbol.as_deref(), Some("USDC"));
        assert_eq!(t.amount.as_deref(), Some("5000000"));
        assert_eq!(t.amount_formatted.as_deref(), Some("5"));
        assert_eq!(t.message_id.emitter_address, EMITTER_HEX);
        assert_eq!(t.message_id.sequence, 42);
        assert!(t.destination_tx_hash.is_none());
    }

    #[test]
    fn missing_metadata_does_not_block_the_transfer() {
        let t = transfer();
        assert!(t.token_symbol.is_none());
        assert!(t.amount_formatted.is_some());
    }

    #[test]
    fn a_chain_reference_from_another_namespace_is_an_error_not_a_guess() {
        let mut d = detected();
        d.destination = RawChainRef::new("cctp", 0);
        assert!(to_bridge_transfer(&d, None).is_err());
    }

    #[test]
    fn token_lookup_reads_the_token_chain_and_address() {
        assert_eq!(
            token_lookup(&detected()),
            Some((1, "TokenMint".to_string()))
        );
        let mut d = detected();
        d.token = None;
        assert_eq!(token_lookup(&d), None);
    }

    #[test]
    fn no_vaa_leaves_the_transfer_detected() {
        for raw in [
            json!({}),
            json!({"vaa": {}}),
            json!({"vaa": {"raw": "!!not base64"}}),
        ] {
            let mut t = transfer();
            assert_eq!(apply_wormholescan(&mut t, &raw), Enrichment::NoVaa);
            assert!(matches!(t.status, BridgeStatus::Detected));
        }
    }

    #[test]
    fn a_matching_vaa_raises_the_status_to_pending() {
        let raw = json!({"vaa": {"raw": vaa_b64(1, [9; 32], 42)}});
        let mut t = transfer();
        assert_eq!(apply_wormholescan(&mut t, &raw), Enrichment::Applied);
        assert!(matches!(t.status, BridgeStatus::Pending));
        assert!(t.destination_tx_hash.is_none());
    }

    #[test]
    fn a_known_destination_tx_completes_the_transfer() {
        let raw = json!({
            "vaa": {"raw": vaa_b64(1, [9; 32], 42)},
            "globalTx": {"destinationTx": {"txHash": "0xdest"}},
        });
        let mut t = transfer();
        assert_eq!(apply_wormholescan(&mut t, &raw), Enrichment::Applied);
        assert!(matches!(t.status, BridgeStatus::Completed));
        assert_eq!(t.destination_tx_hash.as_deref(), Some("0xdest"));
        assert!(t.destination_explorer_url.is_some());
    }

    #[test]
    fn a_vaa_for_another_message_is_ignored() {
        for (chain, emitter, sequence) in [(1, [9; 32], 43), (1, [8; 32], 42), (2, [9; 32], 42)] {
            let raw = json!({
                "vaa": {"raw": vaa_b64(chain, emitter, sequence)},
                "globalTx": {"destinationTx": {"txHash": "0xdest"}},
            });
            let mut t = transfer();
            assert_eq!(apply_wormholescan(&mut t, &raw), Enrichment::Mismatch);
            assert!(matches!(t.status, BridgeStatus::Detected));
            assert!(t.destination_tx_hash.is_none());
        }
    }

    #[test]
    fn destination_tx_is_read_from_target_chain() {
        let raw = json!({"targetChain": {"transaction": {"txHash": "0xtarget"}}});
        assert_eq!(
            destination_tx_from_operation(&raw).as_deref(),
            Some("0xtarget")
        );
    }

    #[test]
    fn target_chain_wins_over_legacy_paths() {
        let raw = json!({
            "targetChain": {"transaction": {"txHash": "0xtarget"}},
            "globalTx": {"destinationTx": {"txHash": "0xlegacy"}},
        });
        assert_eq!(
            destination_tx_from_operation(&raw).as_deref(),
            Some("0xtarget")
        );
    }

    #[test]
    fn empty_or_absent_destination_tx_is_none() {
        for raw in [
            json!({}),
            json!({"targetChain": {}}),
            json!({"targetChain": {"transaction": {"txHash": ""}}}),
        ] {
            assert_eq!(destination_tx_from_operation(&raw), None);
        }
    }

    #[test]
    fn an_operation_with_target_chain_completes_the_transfer() {
        let raw = json!({
            "vaa": {"raw": vaa_b64(1, [9; 32], 42)},
            "targetChain": {"transaction": {"txHash": "0xdest"}},
        });
        let mut t = transfer();
        assert_eq!(apply_wormholescan(&mut t, &raw), Enrichment::Applied);
        assert!(matches!(t.status, BridgeStatus::Completed));
        assert_eq!(t.destination_tx_hash.as_deref(), Some("0xdest"));
    }

    #[test]
    fn apply_destination_tx_completes_only_a_pending_transfer() {
        let mut detected_only = transfer();
        assert!(!apply_destination_tx(&mut detected_only, "0xdest"));
        assert!(matches!(detected_only.status, BridgeStatus::Detected));

        let mut pending = transfer();
        pending.status = BridgeStatus::Pending;
        assert!(apply_destination_tx(&mut pending, "0xdest"));
        assert!(matches!(pending.status, BridgeStatus::Completed));
        assert_eq!(pending.destination_tx_hash.as_deref(), Some("0xdest"));
    }

    #[test]
    fn a_completed_transfer_is_never_rewritten() {
        let mut t = transfer();
        t.status = BridgeStatus::Pending;
        assert!(apply_destination_tx(&mut t, "0xfirst"));
        assert!(!apply_destination_tx(&mut t, "0xsecond"));
        assert_eq!(t.destination_tx_hash.as_deref(), Some("0xfirst"));
    }

    #[test]
    fn a_blank_hash_changes_nothing() {
        let mut t = transfer();
        t.status = BridgeStatus::Pending;
        assert!(!apply_destination_tx(&mut t, "  "));
        assert!(matches!(t.status, BridgeStatus::Pending));
    }

    #[test]
    fn a_matching_vaa_never_lowers_a_higher_status() {
        let raw = json!({"vaa": {"raw": vaa_b64(1, [9; 32], 42)}});
        let mut t = transfer();
        t.status = BridgeStatus::Completed;
        assert_eq!(apply_wormholescan(&mut t, &raw), Enrichment::Applied);
        assert!(matches!(t.status, BridgeStatus::Completed));
    }
}
