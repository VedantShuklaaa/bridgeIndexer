//! Golden tests: the protocol registry run over real captured transactions.
//!
//! Each fixture is a pair captured by `scripts/capture_fixtures.sh`:
//!   helius_<name>.json    the Solana transaction (what the adapter reads)
//!   wormhole_<name>.json  WormholeScan's own decoding of the same message
//!
//! WormholeScan is the independent reference: the adapter decodes the raw
//! instructions, and its output must agree with what WormholeScan reports.

use bridge::protocols::registry::default_registry;
use bridge::protocols::wormhole::TOKEN_BRIDGE;
use bridge::protocols::{Classification, DetectedTransfer, Direction, Protocol, Reject, SolanaTx};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// Sends out of Solana, decoded from the transaction alone. The two payload-3
/// fixtures reach the Token Bridge through another program (an executor router
/// and a custom program), so the Token Bridge call is an inner instruction.
const OUTBOUND: [&str; 3] = ["completed_outbound", "payload3_executor", "payload3_plain"];
const ROUTED: [&str; 2] = ["payload3_executor", "payload3_plain"];
/// Redeems into Solana: `Unsupported` until A1b.
const INBOUND: [&str; 2] = ["inbound_redeem", "inbound_redeem_tb"];

struct Fixture {
    name: String,
    /// WormholeScan's operation.
    op: Value,
    tx: SolanaTx,
}

fn read(file: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(file);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{file}: invalid json: {e}"))
}

fn fixture(name: &str) -> Fixture {
    let op = read(&format!("wormhole_{name}.json"));
    let raw = read(&format!("helius_{name}.json"));
    // The Solana signature is the source tx for a send and the destination tx
    // for a redeem.
    let side = if op["sourceChain"]["chainId"] == 1 {
        "/sourceChain/transaction/txHash"
    } else {
        "/targetChain/transaction/txHash"
    };
    let signature = op
        .pointer(side)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name}: no Solana signature at {side}"));
    let tx = SolanaTx::from_helius(&raw, signature)
        .unwrap_or_else(|e| panic!("{name}: from_helius failed: {e}"));
    Fixture {
        name: name.to_string(),
        op,
        tx,
    }
}

/// Runs the registry the way the pipeline does. Exactly one adapter must match.
fn classify(f: &Fixture) -> Classification {
    let mut results = default_registry().classify(&f.tx);
    assert_eq!(results.len(), 1, "{}: adapters that matched", f.name);
    let (protocol, classification) = results.remove(0);
    assert_eq!(protocol, Protocol::Wormhole, "{}: protocol", f.name);
    classification
}

fn transfers(f: &Fixture) -> Vec<DetectedTransfer> {
    match classify(f) {
        Classification::Transfers(t) => t,
        other => panic!("{}: expected transfers, got {other:?}", f.name),
    }
}

fn top_level_programs(tx: &SolanaTx) -> Vec<&str> {
    tx.instructions
        .iter()
        .filter(|ix| ix.inner_of.is_none())
        .map(|ix| ix.program_id.as_str())
        .collect()
}

#[test]
fn outbound_transfers_match_wormholescan() {
    for name in OUTBOUND {
        let f = fixture(name);
        let found = transfers(&f);
        assert_eq!(found.len(), 1, "{name}: transfers in the tx");
        let t = &found[0];

        let payload = &f.op["content"]["payload"];
        let std = &f.op["content"]["standarizedProperties"];
        let s = |v: &Value| v.as_str().unwrap_or_default().to_string();

        // identity: emitter chain / emitter address / sequence
        assert_eq!(t.message.key(), s(&f.op["id"]), "{name}: message key");
        assert_eq!(t.direction, Direction::Outbound, "{name}: direction");
        assert_eq!(t.observed_tx, f.tx.signature, "{name}: observed tx");
        assert_eq!(t.event_index, 0, "{name}: event index");
        assert_eq!(t.source.id, "1", "{name}: source chain");
        assert!(t.source_wallet.is_some(), "{name}: source wallet");

        // the transfer itself
        assert_eq!(
            t.destination.id,
            payload["toChain"].to_string(),
            "{name}: destination chain"
        );
        assert_eq!(
            t.amount_raw.as_ref().map(|a| a.to_string()),
            Some(s(&payload["amount"])),
            "{name}: amount"
        );
        let token = t.token.as_ref().unwrap_or_else(|| panic!("{name}: token"));
        assert_eq!(
            token.chain.id,
            payload["tokenChain"].to_string(),
            "{name}: token chain"
        );
        assert!(
            token.address.eq_ignore_ascii_case(&s(&std["tokenAddress"])),
            "{name}: token address {} vs {}",
            token.address,
            s(&std["tokenAddress"])
        );
        let wallet = t.destination_wallet.as_deref().unwrap_or_default();
        assert!(
            wallet.eq_ignore_ascii_case(&s(&std["toAddress"])),
            "{name}: destination wallet {wallet} vs {}",
            s(&std["toAddress"])
        );
    }
}

#[test]
fn routed_transfers_are_found_through_inner_calls() {
    for name in ROUTED {
        let f = fixture(name);
        // the point of these fixtures: no top-level Token Bridge instruction
        assert!(
            !top_level_programs(&f.tx).contains(&TOKEN_BRIDGE),
            "{name}: expected the Token Bridge only as an inner call"
        );
        assert!(
            f.tx.program_ids().contains(TOKEN_BRIDGE),
            "{name}: Token Bridge missing from the tx"
        );
        assert_eq!(transfers(&f).len(), 1, "{name}: transfers");
        // a transfer with payload (id 3), as WormholeScan decodes it
        assert_eq!(f.op["content"]["payload"]["payloadType"], 3, "{name}");
    }
}

#[test]
fn an_attest_call_is_rejected_as_not_a_transfer() {
    let f = fixture("admin_call");
    // payload id 2 is an attest (asset metadata) message
    assert_eq!(f.op["content"]["payload"]["payloadType"], 2);
    assert!(
        matches!(
            classify(&f),
            Classification::Rejected(Reject::NotTransfer(_))
        ),
        "{}: {:?}",
        f.name,
        classify(&f)
    );
}

/// Flip these to expect transfers when A1b lands.
#[test]
fn inbound_redeems_are_unsupported_until_a1b() {
    for name in INBOUND {
        let f = fixture(name);
        assert_eq!(
            f.op["targetChain"]["chainId"], 1,
            "{name}: redeems into Solana"
        );
        assert!(
            matches!(
                classify(&f),
                Classification::Rejected(Reject::Unsupported(_))
            ),
            "{name}: {:?}",
            classify(&f)
        );
    }
}
