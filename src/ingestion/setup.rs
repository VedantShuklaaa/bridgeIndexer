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

fn parse_deploy_block(hex_str: &str) -> u64 {
    u64::from_str_radix(hex_str.trim_start_matches("0x"), 16).unwrap_or(0)
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
