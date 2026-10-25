use super::{EnrichInput, EnrichOutcome, Enriched, EnrichmentProvider, Prefetch};
use crate::chains::ChainRegistry;
use crate::clients::limiter::{Limiter, call_upstream};
use crate::clients::wormhole;
use crate::domain::bridge_transfer::{BridgeMessageId, BridgeStatus, BridgeTransfer};
use crate::error::AppError;
use crate::normaliser;
use crate::services::detect::{self, Enrichment};
use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;

/// WormholeScan: the VAA from `/operations`, then the destination tx from `/global-tx`.
/// Every call goes through the shared WormholeScan limiter.
pub struct WormholeScanProvider {
    client: Client,
    base_url: String,
    limiter: Arc<Limiter>,
    chains: ChainRegistry,
}

impl WormholeScanProvider {
    pub fn new(
        client: Client,
        base_url: String,
        limiter: Arc<Limiter>,
        chains: ChainRegistry,
    ) -> Self {
        Self {
            client,
            base_url,
            limiter,
            chains,
        }
    }
}

/// "WormholeScan does not know this tx yet" is an answer. Anything else means the
/// provider could not be asked.
fn outcome_for_error(error: &AppError) -> EnrichOutcome {
    match error {
        AppError::TransactionNotFound(_) => EnrichOutcome::NoVaa,
        _ => EnrichOutcome::Unavailable,
    }
}

#[async_trait]
impl EnrichmentProvider for WormholeScanProvider {
    fn name(&self) -> &'static str {
        "wormholescan"
    }

    async fn enrich(&self, input: EnrichInput<'_>, transfer: &mut BridgeTransfer) -> Enriched {
        let fetched;
        let operation = match input.prefetch {
            Prefetch::Done(Ok(operation)) => operation,
            Prefetch::Done(Err(error)) => {
                tracing::debug!(signature = %input.source_tx_hash, %error, "wormholescan unavailable, keeping on-chain data");
                return Enriched::of(outcome_for_error(error));
            }
            Prefetch::None => {
                let result = call_upstream(&self.limiter, || {
                    wormhole::get_operation_by_tx_hash(
                        &self.client,
                        &self.base_url,
                        input.source_tx_hash,
                    )
                })
                .await;
                match result {
                    Ok(operation) => {
                        fetched = operation;
                        &fetched
                    }
                    Err(error) => {
                        tracing::debug!(signature = %input.source_tx_hash, %error, "wormholescan unavailable, keeping on-chain data");
                        return Enriched::of(outcome_for_error(&error));
                    }
                }
            }
        };

        let chains = self.chains.snapshot();
        match detect::apply_wormholescan(transfer, operation, &chains) {
            Enrichment::Applied => {
                // VAA matched but /operations carried no destination tx: ask global-tx,
                // keyed by the on-chain message id.
                if matches!(transfer.status, BridgeStatus::Pending)
                    && let Some(hash) = self.destination_tx(&transfer.message_id).await
                {
                    detect::apply_destination_tx(transfer, &hash, &chains);
                }
                Enriched {
                    outcome: EnrichOutcome::Applied,
                    bridge_event: normaliser::wormhole::normalise(operation.clone()).ok(),
                }
            }
            Enrichment::NoVaa => Enriched::of(EnrichOutcome::NoVaa),
            Enrichment::Mismatch => Enriched::of(EnrichOutcome::Mismatch),
        }
    }

    /// Best effort: a failure is logged and treated as "not known yet", it never fails
    /// the analysis.
    async fn destination_tx(&self, id: &BridgeMessageId) -> Option<String> {
        call_upstream(&self.limiter, || {
            wormhole::get_destination_tx(&self.client, &self.base_url, id)
        })
        .await
        .inspect_err(|error| tracing::debug!(%error, "global-tx lookup failed"))
        .ok()
        .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::bridge_transfer::ChainId;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use governor::{Quota, RateLimiter};
    use serde_json::json;
    use std::num::NonZeroU32;

    fn provider() -> WormholeScanProvider {
        // Nothing here may touch the network: every test passes a prefetched response
        // that is decided without a `global-tx` call.
        WormholeScanProvider::new(
            Client::new(),
            "http://127.0.0.1:1".into(),
            Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(100).unwrap(),
            ))),
            ChainRegistry::default(),
        )
    }

    fn transfer() -> BridgeTransfer {
        BridgeTransfer {
            source_chain: ChainId::Solana,
            source_tx_hash: Some("sig".into()),
            source_wallet: None,
            source_explorer_url: None,
            destination_chain: ChainId::Ethereum,
            destination_wallet: None,
            destination_tx_hash: None,
            destination_explorer_url: None,
            token: None,
            token_symbol: None,
            amount: None,
            amount_formatted: None,
            message_id: BridgeMessageId {
                emitter_chain: 1,
                emitter_address: "09".repeat(32),
                sequence: 42,
            },
            status: BridgeStatus::Detected,
        }
    }

    fn vaa_b64(sequence: u64) -> String {
        let mut bytes = vec![1u8, 0, 0, 0, 0, 0];
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&[9u8; 32]);
        bytes.extend_from_slice(&sequence.to_be_bytes());
        bytes.push(1);
        STANDARD.encode(bytes)
    }

    async fn run(prefetch: Prefetch<'_>, transfer: &mut BridgeTransfer) -> EnrichOutcome {
        provider()
            .enrich(
                EnrichInput {
                    source_tx_hash: "sig",
                    prefetch,
                },
                transfer,
            )
            .await
            .outcome
    }

    #[tokio::test]
    async fn a_known_destination_tx_completes_the_transfer() {
        let op = json!({
            "vaa": {"raw": vaa_b64(42)},
            "targetChain": {"transaction": {"txHash": "0xdest"}},
        });
        let mut t = transfer();
        assert_eq!(
            run(Prefetch::Done(Ok(&op)), &mut t).await,
            EnrichOutcome::Applied
        );
        assert!(matches!(t.status, BridgeStatus::Completed));
        assert_eq!(t.destination_tx_hash.as_deref(), Some("0xdest"));
    }

    #[tokio::test]
    async fn a_vaa_for_another_message_changes_nothing() {
        let op = json!({"vaa": {"raw": vaa_b64(43)}});
        let mut t = transfer();
        assert_eq!(
            run(Prefetch::Done(Ok(&op)), &mut t).await,
            EnrichOutcome::Mismatch
        );
        assert!(matches!(t.status, BridgeStatus::Detected));
    }

    #[tokio::test]
    async fn no_vaa_yet_leaves_the_transfer_detected() {
        let op = json!({});
        let mut t = transfer();
        assert_eq!(
            run(Prefetch::Done(Ok(&op)), &mut t).await,
            EnrichOutcome::NoVaa
        );
        assert!(matches!(t.status, BridgeStatus::Detected));
    }

    #[tokio::test]
    async fn not_found_is_an_answer_but_other_errors_are_not() {
        let not_found = AppError::TransactionNotFound("sig".into());
        let rate_limited = AppError::UpstreamRateLimited("wormhole");

        let mut t = transfer();
        assert_eq!(
            run(Prefetch::Done(Err(&not_found)), &mut t).await,
            EnrichOutcome::NoVaa
        );
        assert_eq!(
            run(Prefetch::Done(Err(&rate_limited)), &mut t).await,
            EnrichOutcome::Unavailable
        );
        assert!(matches!(t.status, BridgeStatus::Detected));
    }

    #[tokio::test]
    async fn a_completed_transfer_is_never_moved_backwards() {
        let op = json!({"vaa": {"raw": vaa_b64(42)}});
        let mut t = transfer();
        t.status = BridgeStatus::Completed;
        t.destination_tx_hash = Some("0xfirst".into());
        run(Prefetch::Done(Ok(&op)), &mut t).await;
        assert!(matches!(t.status, BridgeStatus::Completed));
        assert_eq!(t.destination_tx_hash.as_deref(), Some("0xfirst"));
    }
}
