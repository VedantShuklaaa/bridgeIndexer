use crate::chain_adapters::registry::AdapterRegistry;
use crate::domain::bridge_transfer::{BridgeMessageId, BridgeStatus, BridgeTransfer, ChainId};
use crate::error::AppError;
use crate::services::token_metadata::{TokenMetadata, resolve_token_metadata};
use crate::util::explorer::explorer_tx_url;
use crate::vaa::decode::format_amount;

pub struct CorrelateParams {
    pub source_tx_hash: String,
    pub source_wallet: Option<String>,
    pub message_id: BridgeMessageId,
    pub destination_chain_id: u16,
    pub token: Option<String>,
    pub token_chain: u16,
    pub symbol_hint: Option<String>,
    pub raw_amount: Option<u128>,
    pub amount: Option<String>,
    pub destination_wallet: Option<String>,
    pub known_destination_tx: Option<String>,
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
        token_chain,
        symbol_hint,
        raw_amount,
        amount,
        destination_wallet,
        known_destination_tx,
    } = params;

    let destination_chain = ChainId::from_wormhole_id(destination_chain_id);

    // Destination completion is read straight off the destination chain —
    // EvmAdapter::find_transaction pages eth_getLogs for the token bridge's
    // TransferRedeemed event. No external indexer in the loop: if we don't
    // have a chain adapter for the destination yet, or the log scan hasn't
    // turned it up, the transfer is just Pending until it does.
    let (destination_tx_hash, status) = if let Some(known) = known_destination_tx {
        (Some(known), BridgeStatus::Completed)
    } else {
        match registry.get_evm(destination_chain_id) {
            Some(adapter) => match adapter.find_transaction(&message_id).await? {
                Some(info) => (Some(info.tx_hash), BridgeStatus::Completed),
                None => (None, BridgeStatus::Pending),
            },
            None => (None, BridgeStatus::Detected),
        }
    };

    let effective_raw_amount: Option<u128> =
        raw_amount.or_else(|| amount.as_ref().and_then(|a| a.parse::<u128>().ok()));

    let metadata = match &token {
        Some(token_addr) => {
            resolve_token_metadata(registry, token_chain, token_addr, symbol_hint)
                .await?
        }
        None => TokenMetadata {
            symbol: None,
            decimals: None,
        },
    };

    let amount_formatted = effective_raw_amount.map(|r| format_amount(r, metadata.decimals));

    let source_explorer_url = explorer_tx_url(message_id.emitter_chain, &source_tx_hash);
    let destination_explorer_url = destination_tx_hash
        .as_ref()
        .and_then(|h| explorer_tx_url(destination_chain_id, h));

    Ok(BridgeTransfer {
        source_chain: ChainId::from_wormhole_id(message_id.emitter_chain),
        source_tx_hash,
        source_wallet,
        source_explorer_url,
        destination_chain,
        destination_wallet,
        destination_tx_hash,
        destination_explorer_url,
        token,
        token_symbol: metadata.symbol,
        amount: amount.or_else(|| raw_amount.map(|r| r.to_string())),
        amount_formatted,
        message_id,
        status,
    })
}
