//! The admin API's HTTP side: `POST /api/admin/{name}` with JSON arguments
//! (see [`crate::api`]), for the signed-in organizer only.

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use dioxus_clerk::server::ClerkAuth;

use super::{AppState, auth, service::Error};
use crate::api::{self, ApiError, DispatchError};

pub fn routes() -> Router {
    Router::new().route("/api/admin/{name}", post(call))
}

/// `ClerkAuthLayer` (around the whole router) verifies the session; without one,
/// or without the layer, `auth` is `None` and only dev auth lets the call through.
async fn call(
    Extension(app): Extension<AppState>,
    auth: Option<ClerkAuth>,
    Path(name): Path<String>,
    args: Bytes,
) -> Response {
    let Some(owner) = auth::owner(&app, auth) else {
        return error(StatusCode::UNAUTHORIZED, "Please sign in.");
    };
    match api::dispatch(&app, &owner, &name, &args).await {
        Ok(value) => ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response(),
        Err(e) => app_error(&name, e),
    }
}

/// The status and the organizer's message for why a call failed.
fn app_error(name: &str, e: DispatchError) -> Response {
    let status = match &e {
        DispatchError::Unknown | DispatchError::App(Error::NotFound) => StatusCode::NOT_FOUND,
        DispatchError::Arguments(_) => StatusCode::BAD_REQUEST,
        DispatchError::App(Error::Refused(r)) if r.invalid() => StatusCode::BAD_REQUEST,
        DispatchError::App(Error::Refused(_)) => StatusCode::CONFLICT,
        DispatchError::App(Error::Unavailable) => StatusCode::SERVICE_UNAVAILABLE,
        DispatchError::App(Error::Internal(e)) => {
            tracing::error!(function = name, error = %e, "internal error");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    };
    error(status, &e.to_string())
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(ApiError::new(message))).into_response()
}
