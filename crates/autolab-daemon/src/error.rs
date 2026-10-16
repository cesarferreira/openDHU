//! Uniform error responses. The framework's own failures (unknown route, wrong method, malformed
//! JSON) use the same schema as everything else.

use autolab_core::Error;
use axum::Json;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::api::{ErrorBody, ErrorEnvelope};

pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

fn status_for(e: &Error) -> StatusCode {
    match e {
        Error::InvalidArgs(_) => StatusCode::BAD_REQUEST,
        Error::BackendUnavailable(_) | Error::DeviceDisconnected(_) => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        Error::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
        Error::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
        Error::SessionConflict(_) => StatusCode::CONFLICT,
        Error::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorEnvelope {
            error: ErrorBody::from_error(&self.0),
        };
        (status_for(&self.0), Json(body)).into_response()
    }
}

/// An error that is not one of AutoLab's operation errors (auth, routing).
pub fn plain(status: StatusCode, code: &str, message: &str) -> Response {
    let body = ErrorEnvelope {
        error: ErrorBody {
            code: code.into(),
            message: message.into(),
            exit_code: 1,
        },
    };
    (status, Json(body)).into_response()
}

pub async fn not_found() -> Response {
    plain(StatusCode::NOT_FOUND, "not_found", "no such endpoint")
}

pub async fn method_not_allowed() -> Response {
    plain(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "this endpoint does not accept that method",
    )
}

/// `Json` extractor whose rejections use the error schema.
pub struct AppJson<T>(pub T);

impl<S, T> FromRequest<S> for AppJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(Self(v)),
            Err(rej) => Err(ApiError(Error::InvalidArgs(format!(
                "invalid request body: {}",
                rej.body_text()
            )))),
        }
    }
}
