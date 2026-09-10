use reqwest::Client;
use sqlx::PgPool;

use crate::config::AppConfig;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub http_client: Client,
    pub config: AppConfig,
}

impl AppState {
    pub fn new(db: PgPool, config: AppConfig) -> anyhow::Result<Self> {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;

        Ok(Self {
            db,
            http_client,
            config,
        })
    }
}
