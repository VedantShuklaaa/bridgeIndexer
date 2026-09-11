use crate::chain_adapters::registry::AdapterRegistry;
use crate::error::AppError;

pub struct TokenMetadata {
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
}

pub async fn resolve_token_metadata(
    registry: &AdapterRegistry,
    token_chain: u16,
    token_address: &str,
    wormholescan_symbol_hint: Option<String>,
) -> Result<TokenMetadata, AppError> {
    let (symbol, decimals) = match registry.get_token_metadata(token_chain) {
        Some(adapter) => {
            let decimals = match adapter.token_decimals(token_address).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(token = %token_address, token_chain, error = %e, "token_decimals failed");
                    None
                }
            };
            let symbol = match adapter.token_symbol(token_address).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(token = %token_address, token_chain, error = %e, "token_symbol failed");
                    None
                }
            };
            (symbol, decimals)
        }
        None => (None, None),
    };

    Ok(TokenMetadata {
        symbol: symbol.or(wormholescan_symbol_hint), // fallback only, never authoritative
        decimals,
    })
}
