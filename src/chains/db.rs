//! Loads the chain registry from Postgres.

use super::{ChainRegistry, ChainRow, MappingRow, RegistryStatus, Snapshot};
use anyhow::Result;
use sqlx::PgPool;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

type ChainTuple = (i16, String, String, String, Option<String>, String);
type MappingTuple = (String, String, i16, String);

/// Reads both tables and builds a fresh [`Snapshot`].
///
/// A row whose `status` is not a known value is skipped with a warning rather than
/// failing the load, and it never resolves.
pub async fn load_snapshot(pool: &PgPool) -> Result<Snapshot> {
    let chains: Vec<ChainTuple> = sqlx::query_as(
        "SELECT id, slug, name, kind, explorer_tx_url, status FROM chains ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    let mappings: Vec<MappingTuple> = sqlx::query_as(
        "SELECT protocol, protocol_chain_id, chain_id, status
         FROM protocol_chain_ids
         ORDER BY protocol, protocol_chain_id",
    )
    .fetch_all(pool)
    .await?;

    let chains = chains
        .into_iter()
        .filter_map(|(id, slug, name, kind, explorer_tx_url, status)| {
            let Some(status) = RegistryStatus::parse(&status) else {
                warn!(chain = %slug, %status, "skipping chain with unknown status");
                return None;
            };
            Some(ChainRow {
                id,
                slug,
                name,
                kind,
                explorer_tx_url,
                status,
            })
        })
        .collect();

    let mappings = mappings
        .into_iter()
        .filter_map(|(protocol, protocol_chain_id, chain_id, status)| {
            let Some(status) = RegistryStatus::parse(&status) else {
                warn!(%protocol, id = %protocol_chain_id, %status, "skipping mapping with unknown status");
                return None;
            };
            Some(MappingRow {
                protocol,
                protocol_chain_id,
                chain_id,
                status,
            })
        })
        .collect();

    Ok(Snapshot::build(chains, mappings))
}

impl ChainRegistry {
    /// Builds a registry from the database.
    pub async fn load(pool: &PgPool) -> Result<Self> {
        Ok(Self::new(load_snapshot(pool).await?))
    }

    /// Replaces the snapshot with the current database state. On error the previous
    /// snapshot stays in place, so a database blip never empties the registry.
    pub async fn refresh(&self, pool: &PgPool) -> Result<()> {
        self.replace(load_snapshot(pool).await?);
        Ok(())
    }
}

/// Refreshes the registry every `interval` until `stop` is cancelled, so a manual
/// `INSERT` or `UPDATE` takes effect without a restart. A failed refresh is logged
/// and retried on the next tick.
pub async fn run_refresh_loop(
    registry: ChainRegistry,
    pool: PgPool,
    interval: Duration,
    stop: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
        }
        match registry.refresh(&pool).await {
            Ok(()) => info!("chain registry refreshed"),
            Err(error) => warn!(%error, "chain registry refresh failed, keeping previous snapshot"),
        }
    }
}
