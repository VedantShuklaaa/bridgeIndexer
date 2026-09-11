pub fn explorer_tx_url(chain_id: u16, tx_hash: &str) -> Option<String> {
    match chain_id {
        1 => Some(format!("https://explorer.solana.com/tx/{tx_hash}")),
        2 => Some(format!("https://etherscan.io/tx/{tx_hash}")),
        4 => Some(format!("https://bscscan.com/tx/{tx_hash}")),
        5 => Some(format!("https://polygonscan.com/tx/{tx_hash}")),
        6 => Some(format!("https://snowtrace.io/tx/{tx_hash}")),
        23 => Some(format!("https://arbiscan.io/tx/{tx_hash}")),
        24 => Some(format!("https://optimistic.etherscan.io/tx/{tx_hash}")),
        25 => Some(format!("https://gnosisscan.io/tx/{tx_hash}")),
        30 => Some(format!("https://basescan.org/tx/{tx_hash}")),
        _ => None, // no known explorer for this chain yet
    }
}
