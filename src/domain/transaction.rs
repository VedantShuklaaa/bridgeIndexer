use super::bridge::BridgeEvent;
use super::bridge_transfer::BridgeTransfer;
use crate::decoding::evm_decode::DecodedAction; // new top-level module: src/decoding/evm_decode.rs — add `pub mod decoding;` in main.rs/lib.rs and `pub mod evm_decode;` in src/decoding/mod.rs
use serde::Serialize;

#[derive(Debug, Serialize, Clone, Copy)]
pub enum Chain {
    Solana,
    Ethereum,
    Bsc,
    Polygon,
    Avalanche,
    Arbitrum,
    Optimism,
    Gnosis,
    Base,
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
}

impl Chain {
    pub fn wormhole_id(&self) -> u16 {
        match self {
            Chain::Solana => 1,
            Chain::Ethereum => 2,
            Chain::Bsc => 4,
            Chain::Polygon => 5,
            Chain::Avalanche => 6,
            Chain::Kaia => 13,
            Chain::Celo => 14,
            Chain::Moonbeam => 16,
            Chain::Arbitrum => 23,
            Chain::Optimism => 24,
            Chain::Gnosis => 25,
            Chain::Base => 30,
            Chain::Seievm => 32,
            Chain::Scroll => 34,
            Chain::Linea => 38,
            Chain::Berachain => 39,
            Chain::SnaxChain => 43,
            Chain::Unichain => 44,
            Chain::Worldchain => 45,
            Chain::Ink => 46,
            Chain::HyperEvm => 47,
            Chain::Monad => 48,
            Chain::Mezo => 50,
            Chain::Fogo => 51,
            Chain::Sonic => 52,
            Chain::Converge => 53,
            Chain::Plume => 55,
            Chain::XrplEvm => 57,
            Chain::Moca => 63,
            Chain::MegaEth => 64,
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "solana" => Some(Chain::Solana),
            "ethereum" => Some(Chain::Ethereum),
            "bsc" => Some(Chain::Bsc),
            "polygon" => Some(Chain::Polygon),
            "avalanche" => Some(Chain::Avalanche),
            "kaia" => Some(Chain::Kaia),
            "celo" => Some(Chain::Celo),
            "moonbeam" => Some(Chain::Moonbeam),
            "arbitrum" => Some(Chain::Arbitrum),
            "optimism" => Some(Chain::Optimism),
            "gnosis" => Some(Chain::Gnosis),
            "base" => Some(Chain::Base),
            "seievm" => Some(Chain::Seievm),
            "scroll" => Some(Chain::Scroll),
            "linea" => Some(Chain::Linea),
            "berachain" => Some(Chain::Berachain),
            "snaxchain" => Some(Chain::SnaxChain),
            "unichain" => Some(Chain::Unichain),
            "worldchain" => Some(Chain::Worldchain),
            "ink" => Some(Chain::Ink),
            "hyperevm" => Some(Chain::HyperEvm),
            "monad" => Some(Chain::Monad),
            "mezo" => Some(Chain::Mezo),
            "fogo" => Some(Chain::Fogo),
            "sonic" => Some(Chain::Sonic),
            "converge" => Some(Chain::Converge),
            "plume" => Some(Chain::Plume),
            "xrplevm" => Some(Chain::XrplEvm),
            "moca" => Some(Chain::Moca),
            "megaeth" => Some(Chain::MegaEth),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub enum TxStatus {
    Success,
    Failed,
}

#[derive(Debug, Serialize)]
pub struct NormalisedTransaction {
    pub hash: String,
    pub chain: Chain,
    pub status: TxStatus,
    pub slot: u64, // block number for EVM chains, slot for Solana
    pub timestamp: Option<i64>,
    pub fee_lamports: u64, // wei-denominated fee for EVM chains
    pub signer: Option<String>,

    // Wormhole-specific — populated only by the bridge-transfer pipeline.
    // Left completely untouched by generic EVM decoding.
    pub bridge_event: Option<BridgeEvent>,
    pub bridge_transfer: Option<BridgeTransfer>,

    // Generic, chain-agnostic EVM decoding — native transfer, ERC-20/721/1155
    // events, and arbitrary contract calls (when an ABI is supplied). Empty
    // for non-EVM chains or when nothing decodable was found; never used to
    // signal failure (see `decode_transaction`'s doc comment).
    pub decoded_actions: Vec<DecodedAction>,
}
