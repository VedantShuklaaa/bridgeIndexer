#[derive(Clone, Debug)]
pub struct AppConfig {
    pub database_url: String,
    pub helius_url: String,   
    pub wormhole_url: String, 
    pub port: u16,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok(); // no-op in prod if .env absent

        Ok(Self {
            database_url: std::env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL not set"))?,
            helius_url: std::env::var("HELIUS_URL")
                .map_err(|_| anyhow::anyhow!("HELIUS_URL not set"))?,
            wormhole_url: std::env::var("WORMHOLE_URL")
                .unwrap_or_else(|_| "https://api.wormholescan.io".to_string()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
        })
    }
}
