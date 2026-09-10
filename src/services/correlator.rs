use crate::chain_adapters::registry::AdapterRegistry;
use crate::domain::bridge_transfer::{BridgeMessageId, BridgeStatus, BridgeTransfer, ChainId};
use crate::error::AppError;

/// Destination info already known from Wormholescan's `operations` response
/// (globalTx.destinationTx), if present — avoids a redundant adapter call
/// when Wormholescan has already observed the redemption.
pub struct KnownDestination {
    pub tx_hash: String,
    pub wallet: Option<String>,
}

pub struct CorrelateParams {
    pub source_tx_hash: String,
    pub source_wallet: Option<String>,
    pub message_id: BridgeMessageId,
    pub destination_chain_id: u16,
    pub token: Option<String>,
    pub amount: Option<String>,
    pub destination_wallet_from_vaa: Option<String>,
    pub known_destination: Option<KnownDestination>,
}

pub async fn correlate(
    registry: &AdapterRegistry,
    params: CorrelateParams,
) -> Result<BridgeTransfer, AppError> {
    let CorrelateParams {
        source_tx_hash,
        source_wallet,
        message_id,
        destination_chain_id,
        token,
        amount,
        destination_wallet_from_vaa,
        known_destination,
    } = params;

    let destination_chain = ChainId::from_wormhole_id(destination_chain_id);

    let (destination_tx_hash, destination_wallet, status) = match known_destination {
        Some(known) => (
            Some(known.tx_hash),
            known.wallet.or(destination_wallet_from_vaa),
            BridgeStatus::Completed,
        ),
        None => {
            let adapter = registry.get(destination_chain_id);
            match adapter {
                None => (None, destination_wallet_from_vaa, BridgeStatus::Detected),
                Some(adapter) => match adapter.find_transaction(&message_id).await? {
                    Some(info) => (
                        Some(info.tx_hash),
                        info.wallet.or(destination_wallet_from_vaa),
                        BridgeStatus::Completed,
                    ),
                    None => (None, destination_wallet_from_vaa, BridgeStatus::Pending),
                },
            }
        }
    };

    Ok(BridgeTransfer {
        source_chain: ChainId::Solana,
        source_tx_hash,
        source_wallet,
        destination_chain,
        destination_wallet,
        destination_tx_hash,
        token,
        amount,
        message_id,
        status,
    })
}
