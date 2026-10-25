//! What one analysis produces: the v1 transaction body plus every transfer decoded from
//! the transaction, in the order the adapters found them. Persistence writes one row per
//! decoded transfer; `/api/v1` keeps reading the first one.

use crate::domain::bridge_transfer::BridgeTransfer;
use crate::domain::transaction::NormalisedTransaction;
use crate::protocols::DetectedTransfer;

#[derive(Debug, Clone)]
pub struct DecodedTransfer {
    /// The adapter's view: identity, `event_index`, raw chain references, `via`.
    pub detected: DetectedTransfer,
    /// The same transfer as the API and storage shape (status, wallets, amounts).
    pub bridge: BridgeTransfer,
}

#[derive(Debug, Clone)]
pub struct Analysis {
    /// The v1 body for the first transfer. The pipeline keeps it for the cached
    /// analysis column and for `/api/v1`.
    pub tx: NormalisedTransaction,
    /// Every decoded transfer. Empty for the WormholeScan-driven path, which can only
    /// produce one transfer without raw chain references.
    pub transfers: Vec<DecodedTransfer>,
}
