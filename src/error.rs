use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid transaction hash: {0}")]
    InvalidTransactionHash(String),

    #[error("transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("upstream provider error ({provider}): {message}")]
    UpstreamProvider {
        provider: &'static str,
        message: String,
    },

    #[error("upstream provider timed out: {0}")]
    UpstreamTimeout(&'static str),

    #[error("failed to normalise transaction data: {0}")]
    Normalisation(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

/// What actually goes on the wire. Kept separate from `AppError` so internal
/// error detail (e.g. anyhow chains) never leaks to the client.
#[derive(Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl AppError {
    fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            AppError::InvalidTransactionHash(_) => (StatusCode::BAD_REQUEST, "INVALID_TX_HASH"),
            AppError::TransactionNotFound(_) => (StatusCode::NOT_FOUND, "TX_NOT_FOUND"),
            AppError::UpstreamProvider { .. } => (StatusCode::BAD_GATEWAY, "UPSTREAM_ERROR"),
            AppError::UpstreamTimeout(_) => (StatusCode::GATEWAY_TIMEOUT, "UPSTREAM_TIMEOUT"),
            AppError::Normalisation(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "NORMALISATION_FAILED")
            }
            AppError::Config(_) => (StatusCode::INTERNAL_SERVER_ERROR, "CONFIG_ERROR"),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();

        // Log full detail server-side (including anyhow chain) regardless of
        // what's returned to the client.
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        } else {
            tracing::warn!(error = %self, "request rejected");
        }

        let message = match &self {
            // Don't leak internal/config detail to clients.
            AppError::Internal(_) | AppError::Config(_) => "internal server error".to_string(),
            other => other.to_string(),
        };

        let body = ErrorBody {
            error: ErrorDetail { code, message },
        };

        (status, Json(body)).into_response()
    }
}
