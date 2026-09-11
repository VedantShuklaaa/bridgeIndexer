use crate::chain_adapters::registry::AdapterRegistry;
use crate::domain::bridge_transfer::{BridgeMessageId, BridgeStatus, BridgeTransfer, ChainId};
use crate::error::AppError;
use crate::util::explorer::explorer_tx_url;
use crate::vaa::decode::format_amount;

pub struct CorrelateParams {
    pub source_tx_hash: String,
    pub source_wallet: Option<String>,
    pub message_id: BridgeMessageId,
    pub destination_chain_id: u16,
    pub token: Option<String>,
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
        raw_amount,
        amount,
        destination_wallet,
        known_destination_tx,
    } = params;

    let destination_chain = ChainId::from_wormhole_id(destination_chain_id);

    let (destination_tx_hash, status) = if let Some(known) = known_destination_tx {
        (Some(known), BridgeStatus::Completed)
    } else {
        let fast = match registry.get_wormholescan(destination_chain_id) {
            Some(a) => a.find_transaction(&message_id).await?,
            None => None,
        };
        match fast {
            Some(info) => (Some(info.tx_hash), BridgeStatus::Completed),
            None => match registry.get_evm(destination_chain_id) {
                // We have a way to check this chain, and checked — genuinely not redeemed yet.
                Some(a) => match a.find_transaction(&message_id).await? {
                    Some(info) => (Some(info.tx_hash), BridgeStatus::Completed),
                    None => (None, BridgeStatus::Pending),
                },
                // No adapter registered for this chain at all — we never
                // actually checked, so this isn't "pending", it's "unsupported".
                // Kept as Detected rather than silently reporting Pending.
                None => (None, BridgeStatus::Detected),
            },
        }
    };

    let effective_raw_amount: Option<u128> =
        raw_amount.or_else(|| amount.as_ref().and_then(|a| a.parse::<u128>().ok()));

    let (token_symbol, amount_formatted) = match (&token, registry.get_evm(destination_chain_id)) {
        (Some(token_addr), Some(adapter)) => {
            let decimals = adapter.token_decimals(token_addr).await.unwrap_or(None);
            let symbol = match adapter.token_symbol(token_addr).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(token = %token_addr, error = %e, "token_symbol failed");
                    None
                }
            };
            (
                symbol,
                effective_raw_amount.map(|r| format_amount(r, decimals)),
            )
        }
        _ => (None, effective_raw_amount.map(|r| format_amount(r, None))),
    };

    let source_explorer_url = explorer_tx_url(1, &source_tx_hash);
    let destination_explorer_url = destination_tx_hash
        .as_ref()
        .and_then(|h| explorer_tx_url(destination_chain_id, h));

    Ok(BridgeTransfer {
        source_chain: ChainId::Solana,
        source_tx_hash,
        source_wallet,
        source_explorer_url,
        destination_chain,
        destination_wallet,
        destination_tx_hash,
        destination_explorer_url,
        token,
        token_symbol,
        amount: amount.or_else(|| raw_amount.map(|r| r.to_string())),
        amount_formatted,
        message_id,
        status,
    })
}
