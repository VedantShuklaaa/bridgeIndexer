//! Seed a throwaway database with realistic `bridge_transfers` rows for the 3g
//! benchmarks and `EXPLAIN` runs. Never point this at production.
//!
//!   SEED_DATABASE_URL=postgres://bridge:bridge@127.0.0.1:5433/bridge_seed \
//!     cargo run --release --bin seed_transfers -- --yes
//!
//! It refuses to run unless `--yes` is passed, and refuses when
//! `SEED_DATABASE_URL` equals `DATABASE_URL`. It truncates `transactions`,
//! `bridge_transfers` and `wormhole_messages` before inserting, so the target
//! database must be disposable.
//!
//! Only `protocol = 'wormhole'` rows are written. The v2 serializer (unchanged by
//! 3g) only models the Wormhole message identity, so a row for any other protocol
//! would make the list endpoint answer 422, which is not a success under the
//! benchmark rules. Direction, status, chains, tokens, `via` and `created_at`
//! (with deliberate duplicate groups) are mixed. Migrations are applied first.

use anyhow::{Context, bail};
use sqlx::postgres::PgConnectOptions;
use std::str::FromStr;
use std::time::Instant;

const USAGE: &str = "usage: seed_transfers --yes [--rows N]

  --yes       required: truncates transactions, bridge_transfers and
              wormhole_messages in SEED_DATABASE_URL first
  --rows N    rows to insert (default 1000000; one transfer per transaction)
  -h, --help  show this help

Reads SEED_DATABASE_URL. Refuses when it equals DATABASE_URL.";

const DEFAULT_ROWS: i64 = 1_000_000;

struct Args {
    rows: i64,
    yes: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let seed_url = std::env::var("SEED_DATABASE_URL").context("SEED_DATABASE_URL is not set")?;
    if let Ok(database_url) = std::env::var("DATABASE_URL")
        && database_url == seed_url
    {
        bail!(
            "SEED_DATABASE_URL must not equal DATABASE_URL: refusing to touch the application database"
        );
    }
    if !args.yes {
        bail!("refusing to write without --yes: this truncates the target database");
    }
    if args.rows < 1 {
        bail!("--rows must be at least 1");
    }

    let options = PgConnectOptions::from_str(&seed_url)
        .context("SEED_DATABASE_URL is not a valid postgres URL")?;
    print_target(&options);

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&seed_url)
        .await
        .context("connect to SEED_DATABASE_URL")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("apply migrations")?;

    let started = Instant::now();
    let mut tx = pool.begin().await?;
    // This is a throwaway database; cross-row durability during the seed buys
    // nothing and costs a lot at this row count.
    sqlx::query("SET LOCAL synchronous_commit = off")
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "TRUNCATE wormhole_messages, bridge_transfers, transactions RESTART IDENTITY CASCADE",
    )
    .execute(&mut *tx)
    .await
    .context("truncate the seed tables")?;

    let transactions = sqlx::query(
        r#"
        INSERT INTO transactions (
            hash, chain, status, slot, timestamp, fee_lamports, signer, created_at
        )
        SELECT 'seed-tx-' || i,
               1,
               'Success',
               300000000 + i,
               (NOW() - INTERVAL '90 days') + ((i - 1) * INTERVAL '7.7 seconds'),
               5000,
               'SeedSigner' || (i % 1000),
               NOW() - (((i - 1) / 4) * INTERVAL '20 seconds')
        FROM generate_series(1, $1) AS g(i)
        "#,
    )
    .bind(args.rows)
    .execute(&mut *tx)
    .await
    .context("insert transactions")?
    .rows_affected();
    println!("inserted {transactions} transactions");

    let transfers = sqlx::query(
        r#"
        WITH p AS (
            SELECT g.i,
                   (g.i % 100) < 35 AS inbound,
                   (ARRAY[2,3,4,5,6,7,8,9,10]::smallint[])[1 + (g.i % 9)] AS other_chain,
                   (ARRAY['2','4','5','6','15','23','24','25','30']::text[])
                       [1 + (g.i % 9)] AS other_ref,
                   CASE
                       WHEN (g.i % 100) < 70 THEN 'Completed'
                       WHEN (g.i % 100) < 85 THEN 'Pending'
                       WHEN (g.i % 100) < 95 THEN 'Detected'
                       WHEN (g.i % 100) < 98 THEN 'Failed'
                       ELSE 'Refunded'
                   END AS status
            FROM generate_series(1, $1) AS g(i)
        ),
        base AS (
            SELECT p.*,
                   CASE WHEN inbound THEN other_ref ELSE '1' END AS src_ref,
                   CASE WHEN inbound THEN other_chain ELSE 1::smallint END AS src_chain,
                   CASE WHEN inbound THEN '1' ELSE other_ref END AS dst_ref,
                   CASE WHEN inbound THEN 1::smallint ELSE other_chain END AS dst_chain,
                   -- Four rows share every created_at, so keyset paging must rely
                   -- on the id tiebreaker.
                   NOW() - (((i - 1) / 4) * INTERVAL '20 seconds') AS created_at
            FROM p
        )
        INSERT INTO bridge_transfers (
            transaction_id, source_tx_hash, source_wallet, source_explorer_url,
            destination_wallet, destination_tx_hash, destination_explorer_url,
            token, token_symbol, amount, amount_formatted, status, created_at, updated_at,
            protocol, message_key, direction, event_index, observed_tx_hash,
            via, via_ref,
            source_ref_protocol, source_ref_id, source_chain_id,
            destination_ref_protocol, destination_ref_id, destination_chain_id,
            token_ref_protocol, token_ref_id, token_chain_id
        )
        SELECT t.id,
               CASE WHEN b.inbound THEN NULL ELSE t.hash END,
               CASE WHEN b.inbound THEN NULL ELSE 'SeedSender' || (b.i % 1000) END,
               NULL,
               'SeedRecipient' || (b.i % 5000),
               CASE
                   WHEN b.inbound THEN t.hash
                   WHEN b.status = 'Completed' THEN 'seed-dst-' || b.i
                   ELSE NULL
               END,
               NULL,
               CASE (b.i % 4)
                   WHEN 0 THEN 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'
                   WHEN 1 THEN 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB'
                   WHEN 2 THEN 'So11111111111111111111111111111111111111112'
                   ELSE NULL
               END,
               CASE (b.i % 4)
                   WHEN 0 THEN 'USDC'
                   WHEN 1 THEN 'USDT'
                   WHEN 2 THEN 'SOL'
                   ELSE NULL
               END,
               CASE WHEN (b.i % 20) = 0 THEN NULL ELSE b.i::numeric * 1000000 END,
               CASE
                   WHEN (b.i % 20) = 0 THEN NULL
                   ELSE ((b.i % 100000)::numeric / 100)::text
               END,
               b.status,
               b.created_at,
               b.created_at,
               'wormhole',
               '1/seed-emitter-' || lpad((b.i % 1000)::text, 4, '0') || '/' || b.i,
               CASE WHEN b.inbound THEN 'Inbound' ELSE 'Outbound' END,
               0,
               t.hash,
               CASE WHEN (b.i % 7) = 0 THEN 'mayan' ELSE NULL END,
               CASE WHEN (b.i % 7) = 0 THEN 'order-' || b.i ELSE NULL END,
               'wormhole', b.src_ref, b.src_chain,
               'wormhole', b.dst_ref, b.dst_chain,
               CASE WHEN (b.i % 4) < 3 THEN 'wormhole' ELSE NULL END,
               CASE WHEN (b.i % 4) < 3 THEN '1' ELSE NULL END,
               CASE WHEN (b.i % 4) < 3 THEN 1::smallint ELSE NULL END
        FROM base b
        JOIN transactions t ON t.hash = 'seed-tx-' || b.i
        "#,
    )
    .bind(args.rows)
    .execute(&mut *tx)
    .await
    .context("insert bridge_transfers")?
    .rows_affected();
    println!("inserted {transfers} bridge_transfers");

    let messages = sqlx::query(
        r#"
        INSERT INTO wormhole_messages (transfer_id, emitter_chain, emitter_address, sequence)
        SELECT b.id, 1, split_part(b.message_key, '/', 2),
               split_part(b.message_key, '/', 3)::numeric
        FROM bridge_transfers b
        WHERE b.protocol = 'wormhole'
        "#,
    )
    .execute(&mut *tx)
    .await
    .context("insert wormhole_messages")?
    .rows_affected();
    println!("inserted {messages} wormhole_messages");

    tx.commit().await?;

    for table in ["transactions", "bridge_transfers", "wormhole_messages"] {
        sqlx::query(&format!("ANALYZE {table}"))
            .execute(&pool)
            .await
            .with_context(|| format!("analyze {table}"))?;
    }

    println!("seeded {} rows in {:.1?}", args.rows, started.elapsed());
    Ok(())
}

fn print_target(options: &PgConnectOptions) {
    let host = options.get_host();
    let port = options.get_port();
    let database = options.get_database().unwrap_or("<none>");
    // Never print the password.
    println!("target: postgres://{host}:{port}/{database}");
    println!("this truncates transactions, bridge_transfers and wormhole_messages");
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        rows: DEFAULT_ROWS,
        yes: false,
    };
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--yes" => args.yes = true,
            "--rows" => {
                let value = argv.next().context("--rows needs a value")?;
                args.rows = value
                    .parse::<i64>()
                    .with_context(|| format!("--rows must be an integer, got {value}"))?;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}\n\n{USAGE}"),
        }
    }
    Ok(args)
}
