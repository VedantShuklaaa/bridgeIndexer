use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use super::solana::SolanaIngester;

async fn wait_until(clients: &mut watch::Receiver<usize>, pred: impl Fn(usize) -> bool) -> bool {
    clients.wait_for(|n| pred(*n)).await.is_ok()
}

/// Keeps the Solana ingester idle until the first `/stream` client connects, runs it
/// while anyone is listening, and stops it again once the last client has been gone
/// for `idle_grace` (so a page refresh doesn't tear the subscription down).
///
/// While it is stopped nothing is subscribed, backfilled or published, so it makes
/// no Helius requests at all.
pub async fn run_on_demand(
    ingester: Arc<SolanaIngester>,
    mut clients: watch::Receiver<usize>,
    idle_grace: Duration,
    shutdown: CancellationToken,
) {
    loop {
        let connected = tokio::select! {
            _ = shutdown.cancelled() => return,
            ok = wait_until(&mut clients, |n| n > 0) => ok,
        };
        if !connected {
            return;
        }

        info!("live client connected, starting Solana ingestion");
        let stop = shutdown.child_token();
        let task = tokio::spawn({
            let ingester = ingester.clone();
            let stop = stop.clone();
            async move {
                if let Err(error) = ingester.run(stop).await {
                    error!(%error, "Solana ingester stopped");
                }
            }
        });

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                ok = wait_until(&mut clients, |n| n == 0) => {
                    if !ok { break; }
                }
            }

            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = sleep(idle_grace) => {
                    if *clients.borrow() == 0 { break; }
                }
                ok = wait_until(&mut clients, |n| n > 0) => {
                    if !ok { break; }
                }
            }
        }

        stop.cancel();
        let _ = task.await;
        info!("no live clients left, Solana ingestion stopped");
    }
}
