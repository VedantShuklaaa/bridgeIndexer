use axum::{
    Json,
    http::{HeaderValue, StatusCode, header::RETRY_AFTER},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("invalid transaction hash: {0}")]
    InvalidTransactionHash(String),

    #[error("transaction not found: {0}")]
    TransactionNotFound(String),

    #[error("VAA not available for transaction: {0}")]
    VaaNotAvailable(String),

    #[error("upstream provider error ({provider}): {message}")]
    UpstreamProvider {
        provider: &'static str,
        message: String,
    },

    #[error("upstream provider rate limited: {0}")]
    UpstreamRateLimited(&'static str),

    #[error("upstream provider timed out: {0}")]
    UpstreamTimeout(&'static str),

    #[error("failed to normalise transaction data: {0}")]
    Normalisation(String),

    #[error("invalid request: {0}")]
    BadRequest(String),

    #[error("service overloaded, retry shortly")]
    Overloaded,

    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

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
            AppError::VaaNotAvailable(_) => (StatusCode::NOT_FOUND, "VAA_NOT_AVAILABLE"),
            AppError::UpstreamProvider { .. } => (StatusCode::BAD_GATEWAY, "UPSTREAM_ERROR"),
            AppError::UpstreamRateLimited(_) => {
                (StatusCode::TOO_MANY_REQUESTS, "UPSTREAM_RATE_LIMITED")
            }
            AppError::UpstreamTimeout(_) => (StatusCode::GATEWAY_TIMEOUT, "UPSTREAM_TIMEOUT"),
            AppError::Normalisation(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "NORMALISATION_FAILED")
            }
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            AppError::Overloaded => (StatusCode::SERVICE_UNAVAILABLE, "OVERLOADED"),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();

        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        } else {
            tracing::warn!(error = %self, "request rejected");
        }

        let message = match &self {
            AppError::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        };

        let retry_after = matches!(
            self,
            AppError::Overloaded | AppError::UpstreamRateLimited(_)
        );

        let mut resp = (
            status,
            Json(ErrorBody {
                error: ErrorDetail { code, message },
            }),
        )
            .into_response();

        if retry_after {
            resp.headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from_static("1"));
        }
        resp
    }
}

impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            AppError::UpstreamTimeout("unknown")
        } else if err.status().map(|s| s.as_u16()) == Some(429) {
            AppError::UpstreamRateLimited("unknown")
        } else {
            AppError::UpstreamProvider {
                provider: "unknown",
                message: err.to_string(),
            }
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Normalisation(err.to_string())
    }
}
