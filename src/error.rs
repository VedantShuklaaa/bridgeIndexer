use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid transaction hash: {0}")]
    InvalidTransactionHash(String),

    #[error("transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("VAA not available for transaction: {0}")]
    VaaNotAvailable(String),

    #[error("upstream provider error ({provider}): {message}")]
    UpstreamProvider { provider: String, message: String },

    #[error("upstream provider rate limited: {0}")]
    UpstreamRateLimited(String),

    #[error("upstream provider timed out: {0}")]
    UpstreamTimeout(String),

    #[error("failed to normalise transaction data: {0}")]
    Normalisation(String),

    #[error("invalid request: {0}")]
    BadRequest(String),

    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    /// Convenience for upstream call sites that don't know the provider yet.
    pub fn upstream(provider: &str, message: impl Into<String>) -> Self {
        AppError::UpstreamProvider {
            provider: provider.to_string(),
            message: message.into(),
        }
    }

    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::InvalidTransactionHash(_) => (StatusCode::BAD_REQUEST, "INVALID_TX_HASH"),
            AppError::TransactionNotFound(_) => (StatusCode::NOT_FOUND, "TX_NOT_FOUND"),
            AppError::VaaNotAvailable(_) => (StatusCode::NOT_FOUND, "VAA_NOT_AVAILABLE"),
            AppError::UpstreamProvider { .. } => (StatusCode::BAD_GATEWAY, "UPSTREAM_ERROR"),
            // 502 — the *upstream* is rate-limiting us, not the caller.
            AppError::UpstreamRateLimited(_) => (StatusCode::BAD_GATEWAY, "UPSTREAM_RATE_LIMITED"),
            AppError::UpstreamTimeout(_) => (StatusCode::GATEWAY_TIMEOUT, "UPSTREAM_TIMEOUT"),
            AppError::Normalisation(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "NORMALISATION_FAILED")
            }
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        }
    }
}

// ── IntoResponse ──────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();

        if status.is_server_error() {
            tracing::error!(
                error = %self,
                source = ?std::error::Error::source(&self),
                "internal error",
            );
        } else {
            tracing::warn!(error = %self, "request failed");
        }

        // Never leak internal details to the caller.
        let message = match &self {
            AppError::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        };

        (
            status,
            Json(ErrorBody {
                error: ErrorDetail { code, message },
            }),
        )
            .into_response()
    }
}

// ── From impls ────────────────────────────────────────────────────────────────

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            AppError::UpstreamTimeout("unknown".into())
        } else if err.is_connect() {
            AppError::upstream("unknown", format!("connection failed: {err}"))
        } else {
            // Note: err.status() is the status that *triggered* a decode error,
            // not a transport-level response — rate-limit detection belongs in
            // rpc_call where you actually hold the StatusCode.
            AppError::upstream("unknown", err.to_string())
        }
    }
}

// Narrowly scoped: only for normalising *our own* data structures.
// RPC response parse failures should map to UpstreamProvider at the call site.
impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Normalisation(err.to_string())
    }
}
