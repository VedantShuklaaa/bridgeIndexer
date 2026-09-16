use reqwest::Client;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::chain_adapters::evm::event_topic;
use crate::config::AppConfig;
use crate::ingestion::evm::EvmIngester;
use crate::redis::producer::RedisProducer;

/// One EVM chain's ingestion config — adding a chain means adding one
/// entry to the Vec built in `evm_chain_configs()`, not new code elsewhere.
struct EvmChainConfig {
    name: &'static str,
    ws_url: String,
    rpc_url: String,
    core_bridge_contract: String,
    core_bridge_deploy_block: String,
    block_range: u64,
}

fn parse_deploy_block(value: &str) -> u64 {
    if let Some(hex) = value.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).unwrap_or(0)
    } else {
        value.parse::<u64>().unwrap_or(0)
    }
}

fn evm_chain_configs(config: &AppConfig) -> Vec<EvmChainConfig> {
    vec![
        EvmChainConfig {
            name: "ethereum",
            ws_url: config.eth_ws_url.clone(),
            rpc_url: config.eth_rpc_url.clone(),
            core_bridge_contract: config.eth_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.eth_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "bsc",
            ws_url: config.bsc_ws_url.clone(),
            rpc_url: config.bsc_rpc_url.clone(),
            core_bridge_contract: config.bsc_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.bsc_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "polygon",
            ws_url: config.polygon_ws_url.clone(),
            rpc_url: config.polygon_rpc_url.clone(),
            core_bridge_contract: config.polygon_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.polygon_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "avalanche",
            ws_url: config.avalanche_ws_url.clone(),
            rpc_url: config.avalanche_rpc_url.clone(),
            core_bridge_contract: config.avalanche_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.avalanche_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "arbitrum",
            ws_url: config.arbitrum_ws_url.clone(),
            rpc_url: config.arbitrum_rpc_url.clone(),
            core_bridge_contract: config.arbitrum_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.arbitrum_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "optimism",
            ws_url: config.optimism_ws_url.clone(),
            rpc_url: config.optimism_rpc_url.clone(),
            core_bridge_contract: config.optimism_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.optimism_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "base",
            ws_url: config.base_ws_url.clone(),
            rpc_url: config.base_rpc_url.clone(),
            core_bridge_contract: config.base_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.base_core_bridge_deploy_block.clone(),
            block_range: 10, // Alchemy free-tier eth_getLogs cap
        },
        // Gnosis intentionally omitted — read-only Core Contract, never
        // originates LogMessagePublished (see README roadmap note).
        EvmChainConfig {
            name: "moonbeam",
            ws_url: config.moonbeam_ws_url.clone(),
            rpc_url: config.moonbeam_rpc_url.clone(),
            core_bridge_contract: config.moonbeam_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.moonbeam_core_bridge_deploy_block.clone(),
            block_range: 2000, // adjust down if the RPC provider caps eth_getLogs tighter
        },
        EvmChainConfig {
            name: "celo",
            ws_url: config.celo_ws_url.clone(),
            rpc_url: config.celo_rpc_url.clone(),
            core_bridge_contract: config.celo_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.celo_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "kaia",
            ws_url: config.kaia_ws_url.clone(),
            rpc_url: config.kaia_rpc_url.clone(),
            core_bridge_contract: config.kaia_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.kaia_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "scroll",
            ws_url: config.scroll_ws_url.clone(),
            rpc_url: config.scroll_rpc_url.clone(),
            core_bridge_contract: config.scroll_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.scroll_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "linea",
            ws_url: config.linea_ws_url.clone(),
            rpc_url: config.linea_rpc_url.clone(),
            core_bridge_contract: config.linea_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.linea_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "berachain",
            ws_url: config.berachain_ws_url.clone(),
            rpc_url: config.berachain_rpc_url.clone(),
            core_bridge_contract: config.berachain_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.berachain_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "seievm",
            ws_url: config.seievm_ws_url.clone(),
            rpc_url: config.seievm_rpc_url.clone(),
            core_bridge_contract: config.seievm_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.seievm_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "unichain",
            ws_url: config.unichain_ws_url.clone(),
            rpc_url: config.unichain_rpc_url.clone(),
            core_bridge_contract: config.unichain_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.unichain_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        EvmChainConfig {
            name: "ink",
            ws_url: config.ink_ws_url.clone(),
            rpc_url: config.ink_rpc_url.clone(),
            core_bridge_contract: config.ink_core_bridge_contract.clone(),
            core_bridge_deploy_block: config.ink_core_bridge_deploy_block.clone(),
            block_range: 2000,
        },
        // Sonic intentionally omitted — its Core Contract is a Wormhole
        // "read-only" deployment: it can receive/verify messages but
        // cannot originate them, so it will never emit LogMessagePublished
        // (same situation as Gnosis above).
    ]
}

fn spawn_one(
    chain: EvmChainConfig,
    http_client: Client,
    redis: RedisProducer,
    db: PgPool,
    shutdown: CancellationToken,
) {
    let deploy_block = parse_deploy_block(&chain.core_bridge_deploy_block);
    let topic = event_topic("LogMessagePublished(address,uint64,uint32,bytes,uint8)");

    let ingester = EvmIngester::new(
        chain.name,
        chain.ws_url,
        http_client,
        chain.rpc_url,
        chain.core_bridge_contract,
        topic,
        deploy_block,
        chain.block_range,
        redis,
        db,
        shutdown,
    );

    // EvmIngester::run() already logs chain-tagged errors internally.
    tokio::spawn(async move {
        let _ = ingester.run().await;
    });
}

/// Spawns one EvmIngester per configured EVM chain. Call once from main().
pub fn spawn_evm_ingesters(
    config: &AppConfig,
    http_client: Client,
    redis: RedisProducer,
    db: PgPool,
    shutdown: CancellationToken,
) {
    for chain in evm_chain_configs(config) {
        spawn_one(
            chain,
            http_client.clone(),
            redis.clone(),
            db.clone(),
            shutdown.clone(),
        );
    }
}
