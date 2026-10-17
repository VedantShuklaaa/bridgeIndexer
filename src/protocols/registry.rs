use std::collections::HashMap;
use std::sync::Arc;

use super::{BridgeAdapter, Classification, Protocol, SolanaTx};

/// Routes a transaction to every adapter whose program appears anywhere in it,
/// top-level or inner. An aggregator and its underlying rail both see the tx.
#[derive(Clone, Default)]
pub struct ProtocolRegistry {
    adapters: Vec<Arc<dyn BridgeAdapter>>,
    /// program ID -> indexes into `adapters`
    by_program: HashMap<String, Vec<usize>>,
}

impl ProtocolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an adapter under its own default program IDs.
    pub fn register(&mut self, adapter: Arc<dyn BridgeAdapter>) {
        let ids: Vec<String> = adapter
            .default_program_ids()
            .iter()
            .map(|s| s.to_string())
            .collect();
        self.register_with_programs(adapter, ids);
    }

    /// Registers an adapter under an explicit list (the `SOLANA_BRIDGE_PROGRAMS` override).
    pub fn register_with_programs(
        &mut self,
        adapter: Arc<dyn BridgeAdapter>,
        program_ids: Vec<String>,
    ) {
        let idx = self.adapters.len();
        self.adapters.push(adapter);
        for id in program_ids {
            let slot = self.by_program.entry(id).or_default();
            if !slot.contains(&idx) {
                slot.push(idx);
            }
        }
    }

    /// Every program ID to subscribe to, sorted and deduplicated.
    pub fn program_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.by_program.keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn protocols_for(&self, program_id: &str) -> Vec<Protocol> {
        self.by_program
            .get(program_id)
            .map(|v| v.iter().map(|&i| self.adapters[i].protocol()).collect())
            .unwrap_or_default()
    }

    /// Runs each matching adapter once, in registration order.
    pub fn classify(&self, tx: &SolanaTx) -> Vec<(Protocol, Classification)> {
        let mut hit: Vec<usize> = tx
            .program_ids()
            .into_iter()
            .filter_map(|p| self.by_program.get(p))
            .flatten()
            .copied()
            .collect();
        hit.sort_unstable();
        hit.dedup();
        hit.into_iter()
            .map(|i| (self.adapters[i].protocol(), self.adapters[i].classify(tx)))
            .collect()
    }
}

/// Parses `SOLANA_BRIDGE_PROGRAMS`: a CSV of `protocol:program_id`.
pub fn parse_program_overrides(csv: &str) -> Result<Vec<(Protocol, String)>, String> {
    let mut out = Vec::new();
    for item in csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (proto, id) = item
            .split_once(':')
            .ok_or_else(|| format!("expected protocol:program_id, got {item:?}"))?;
        let protocol =
            Protocol::parse(proto.trim()).ok_or_else(|| format!("unknown protocol {proto:?}"))?;
        let id = id.trim();
        if id.is_empty() {
            return Err(format!("empty program id in {item:?}"));
        }
        out.push((protocol, id.to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocols::{Reject, solana_tx::Instruction};

    struct Fake {
        protocol: Protocol,
        programs: &'static [&'static str],
    }

    impl BridgeAdapter for Fake {
        fn protocol(&self) -> Protocol {
            self.protocol
        }
        fn default_program_ids(&self) -> &'static [&'static str] {
            self.programs
        }
        fn classify(&self, _tx: &SolanaTx) -> Classification {
            Classification::Rejected(Reject::NotTransfer("fake"))
        }
    }

    fn tx_calling(programs: &[(&str, Option<usize>)]) -> SolanaTx {
        SolanaTx {
            signature: "s".into(),
            slot: 1,
            block_time: None,
            signer: None,
            account_keys: vec![],
            instructions: programs
                .iter()
                .map(|(p, inner_of)| Instruction {
                    program_id: p.to_string(),
                    accounts: vec![],
                    data: vec![],
                    parsed: None,
                    inner_of: *inner_of,
                })
                .collect(),
            logs: vec![],
        }
    }

    fn registry() -> ProtocolRegistry {
        let mut r = ProtocolRegistry::new();
        r.register(Arc::new(Fake {
            protocol: Protocol::Wormhole,
            programs: &["WormA", "WormB"],
        }));
        r
    }

    #[test]
    fn subscribes_to_every_default_program() {
        assert_eq!(registry().program_ids(), vec!["WormA", "WormB"]);
    }

    #[test]
    fn routes_on_inner_calls_too() {
        let r = registry();
        let out = r.classify(&tx_calling(&[("Router", None), ("WormB", Some(0))]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, Protocol::Wormhole);
    }

    #[test]
    fn an_adapter_runs_once_even_if_two_of_its_programs_appear() {
        let r = registry();
        let out = r.classify(&tx_calling(&[("WormA", None), ("WormB", None)]));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn unrelated_transactions_match_nothing() {
        assert!(
            registry()
                .classify(&tx_calling(&[("Other", None)]))
                .is_empty()
        );
    }

    #[test]
    fn override_replaces_the_defaults() {
        let mut r = ProtocolRegistry::new();
        r.register_with_programs(
            Arc::new(Fake {
                protocol: Protocol::Wormhole,
                programs: &["WormA"],
            }),
            vec!["Custom".into()],
        );
        assert_eq!(r.program_ids(), vec!["Custom"]);
        assert_eq!(r.protocols_for("Custom"), vec![Protocol::Wormhole]);
        assert!(r.protocols_for("WormA").is_empty());
    }

    #[test]
    fn parses_the_override_list() {
        let v = parse_program_overrides("wormhole:AAA, wormhole:BBB").unwrap();
        assert_eq!(
            v,
            vec![
                (Protocol::Wormhole, "AAA".to_string()),
                (Protocol::Wormhole, "BBB".to_string())
            ]
        );
        assert!(parse_program_overrides("").unwrap().is_empty());
    }

    #[test]
    fn rejects_bad_override_entries() {
        assert!(parse_program_overrides("AAA").is_err());
        assert!(parse_program_overrides("nope:AAA").is_err());
        assert!(parse_program_overrides("wormhole:").is_err());
    }
}
