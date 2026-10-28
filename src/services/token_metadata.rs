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
            // The lookup is shared per registry: it caches per mint and only spends
            // a Helius token a getTransaction does not need right now. Without one
            // (bare registries in tests), ask the adapter directly.
            let resolved = match registry.token_metadata_lookup() {
                Some(lookup) => {
                    lookup
                        .resolve(token_chain, token_address, adapter.as_ref())
                        .await
                }
                None => adapter.token_metadata(token_address).await.map(Some),
            };
            match resolved {
                Ok(Some((decimals, symbol))) => (symbol, decimals),
                // No token was free: best effort, so skip metadata instead of
                // delaying the transfer and do not log a failure.
                Ok(None) => (None, None),
                Err(e) => {
                    tracing::warn!(token = %token_address, token_chain, error = %e, "token metadata failed");
                    (None, None)
                }
            }
        }
        None => (None, None),
    };

    Ok(TokenMetadata {
        symbol: symbol.or(wormholescan_symbol_hint), // fallback only, never authoritative
        decimals,
    })
}
