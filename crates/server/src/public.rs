//! Public JSON endpoints.

use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rafi_store::RaffleRow;

use super::{AppState, service, view};

pub fn routes() -> Router {
    Router::new()
        .route("/api/r/{slug}/view", get(full_view))
        .route("/api/r/{slug}/participant", get(participant))
        .route("/api/r/{slug}/verify.json", get(verify_json))
        .route("/api/altcha", get(challenge))
        .route("/healthz", get(|| async { "ok" }))
}

async fn public_raffle(app: &AppState, slug: &str) -> Result<RaffleRow, StatusCode> {
    match service::public::raffle(app, slug).await {
        Ok(r) => Ok(r),
        Err(service::Error::NotFound) => Err(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::error!(error = %e, "loading raffle");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// The full view: what the projector and the admin show. They poll it.
async fn full_view(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    let raffle = match public_raffle(&app, &slug).await {
        Ok(r) => r,
        Err(status) => return status.into_response(),
    };
    match view::build(&app, &raffle.id).await {
        Ok(Some(v)) => ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "building the view");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The participant view: what phones show. No personal data beyond the
/// drawn entrants' display names (masked when hidden).
async fn participant(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    let raffle = match public_raffle(&app, &slug).await {
        Ok(r) => r,
        Err(status) => return status.into_response(),
    };
    match service::public::participant(&app, &raffle.id).await {
        Ok(Some(p)) => ([(header::CACHE_CONTROL, "no-store")], Json(p.view)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "reading the participant view");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// What verifiers fetch. The raffle's version is the `ETag` (the same `v`,
/// the same content: a name erased later bumps it), and `no-cache` makes
/// browsers and tools revalidate rather than keep a copy, so an erased name
/// is never served from a cache; a finished raffle costs them a 304.
async fn verify_json(
    Extension(app): Extension<AppState>,
    Path(slug): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    match service::verify_data(&app, &slug).await {
        Ok(data) => {
            let etag = format!("\"v{}\"", data.v);
            if headers
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.split(',').any(|t| t.trim() == etag))
            {
                return (
                    StatusCode::NOT_MODIFIED,
                    [
                        (header::ETAG, etag),
                        (header::CACHE_CONTROL, "no-cache".to_owned()),
                    ],
                )
                    .into_response();
            }
            (
                [
                    (header::ETAG, etag),
                    (header::CACHE_CONTROL, "no-cache".to_owned()),
                ],
                Json(data),
            )
                .into_response()
        }
        Err(service::Error::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "verify data");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn challenge(Extension(app): Extension<AppState>) -> Response {
    match altcha_server::axum::challenge_response(&app.policy.proof_of_work) {
        Ok(response) => response,
        Err(e) => {
            tracing::error!(error = %e, "issuing a proof-of-work challenge");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
