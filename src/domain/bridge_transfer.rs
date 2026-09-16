use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub enum ChainId {
    Solana,
    Ethereum,
    Bsc,
    Avalanche,
    Base,
    Arbitrum,
    Optimism,
    Polygon,
    Gnosis,
    Near,
    Moonbeam,
    Celo,
    Kaia,
    Scroll,
    Linea,
    Berachain,
    Seievm,
    SnaxChain,
    Unichain,
    Worldchain,
    Ink,
    HyperEvm,
    Monad,
    Mezo,
    Fogo,
    Sonic,
    Converge,
    Plume,
    XrplEvm,
    Moca,
    MegaEth,
    Unknown(u16),
}

impl ChainId {
    pub fn from_wormhole_id(id: u16) -> Self {
        match id {
            1 => ChainId::Solana,
            2 => ChainId::Ethereum,
            4 => ChainId::Bsc,
            5 => ChainId::Polygon,
            6 => ChainId::Avalanche,
            13 => ChainId::Kaia,
            14 => ChainId::Celo,
            15 => ChainId::Near,
            16 => ChainId::Moonbeam,
            23 => ChainId::Arbitrum,
            24 => ChainId::Optimism,
            25 => ChainId::Gnosis,
            30 => ChainId::Base,
            32 => ChainId::Seievm,
            34 => ChainId::Scroll,
            38 => ChainId::Linea,
            39 => ChainId::Berachain,
            40 => ChainId::Seievm,
            43 => ChainId::SnaxChain,
            44 => ChainId::Unichain,
            45 => ChainId::Worldchain,
            46 => ChainId::Ink,
            47 => ChainId::HyperEvm,
            48 => ChainId::Monad,
            50 => ChainId::Mezo,
            51 => ChainId::Fogo,
            52 => ChainId::Sonic,
            53 => ChainId::Converge,
            55 => ChainId::Plume,
            57 => ChainId::XrplEvm,
            63 => ChainId::Moca,
            64 => ChainId::MegaEth,
            other => ChainId::Unknown(other),
        }
    }

    pub fn wormhole_id(&self) -> u16 {
        match self {
            ChainId::Solana => 1,
            ChainId::Ethereum => 2,
            ChainId::Bsc => 4,
            ChainId::Polygon => 5,
            ChainId::Avalanche => 6,
            ChainId::Kaia => 13,
            ChainId::Celo => 14,
            ChainId::Near => 15,
            ChainId::Moonbeam => 16,
            ChainId::Arbitrum => 23,
            ChainId::Optimism => 24,
            ChainId::Gnosis => 25,
            ChainId::Base => 30,
            ChainId::Seievm => 32,
            ChainId::Scroll => 34,
            ChainId::Linea => 38,
            ChainId::Berachain => 39,
            ChainId::SnaxChain => 43,
            ChainId::Unichain => 44,
            ChainId::Worldchain => 45,
            ChainId::Ink => 46,
            ChainId::HyperEvm => 47,
            ChainId::Monad => 48,
            ChainId::Mezo => 50,
            ChainId::Fogo => 51,
            ChainId::Sonic => 52,
            ChainId::Converge => 53,
            ChainId::Plume => 55,
            ChainId::XrplEvm => 57,
            ChainId::Moca => 63,
            ChainId::MegaEth => 64,
            ChainId::Unknown(id) => *id,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum BridgeStatus {
    Detected,
    Pending,
    Completed,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeMessageId {
    pub emitter_chain: u16,
    pub emitter_address: String,
    pub sequence: u64,
}

#[derive(Debug, Serialize)]
pub struct BridgeTransfer {
    pub source_chain: ChainId,
    pub source_tx_hash: String,
    pub source_wallet: Option<String>,
    pub source_explorer_url: Option<String>,
    pub destination_chain: ChainId,
    pub destination_wallet: Option<String>,
    pub destination_tx_hash: Option<String>,
    pub destination_explorer_url: Option<String>,
    pub token: Option<String>,
    pub token_symbol: Option<String>,
    pub amount: Option<String>, // raw, Wormhole-normalized (unchanged behavior)
    pub amount_formatted: Option<String>, // new — human-readable, decimals-aware
    pub message_id: BridgeMessageId,
    pub status: BridgeStatus,
}
