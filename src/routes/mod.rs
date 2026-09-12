mod v1;

use axum::Router;
use axum::http::{HeaderValue, Method};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::state::AppState;

fn build_cors(allowed_origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| o.parse::<HeaderValue>().ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
}

pub fn build_router(state: AppState) -> Router {
    let cors = build_cors(&state.config.allowed_origins);

    Router::new()
        .nest("/api/v1", v1::router())
        .layer(cors)
        .with_state(state)
}
