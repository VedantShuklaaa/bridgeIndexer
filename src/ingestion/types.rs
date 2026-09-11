use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SolanaRpcRequest<T> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'static str,
    pub params: T,
}

#[derive(Debug, Serialize)]
pub struct LogsFilter {
    pub mentions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LogsConfig {
    pub commitment: &'static str,
}
