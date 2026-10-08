//! Public JSON endpoints.

use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

use super::{AppState, altcha, db, service, view};

pub fn routes() -> Router {
    Router::new()
        .route("/api/r/{slug}/view", get(full_view))
        .route("/api/r/{slug}/participant", get(participant))
        .route("/api/r/{slug}/verify.json", get(verify_json))
        .route("/api/altcha", get(challenge))
        .route("/api/status", get(status))
        .route("/healthz", get(|| async { "ok" }))
}

async fn public_raffle(app: &AppState, slug: &str) -> Result<db::RaffleRow, StatusCode> {
    match db::raffle_by_slug(&app.db, slug).await {
        Ok(Some(r)) if r.state().is_public() => Ok(r),
        Ok(_) => Err(StatusCode::NOT_FOUND),
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
    match rafi_channel::Snapshot::read(&app.db, &raffle.id).await {
        Ok(Some(snap)) => {
            ([(header::CACHE_CONTROL, "no-store")], Json(snap.view())).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "reading the participant view");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn verify_json(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    match service::verify_data(&app, &slug).await {
        Ok(data) => Json(data).into_response(),
        Err(service::AppError::NotFound) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "verify data");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn challenge(Extension(app): Extension<AppState>) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(altcha::challenge(&app)),
    )
        .into_response()
}

/// Number of raffles that are live right now. The deploy pipeline refuses to
/// restart the server while this is non-zero.
async fn status(Extension(app): Extension<AppState>) -> Response {
    let live: Result<(i64,), _> = app
        .db
        .query("SELECT COUNT(*) FROM raffles WHERE state IN ('open', 'frozen', 'drawing') AND test_mode = 0")
        .one()
        .await;
    match live {
        Ok((n,)) => (
            [(header::CACHE_CONTROL, "no-store")],
            Json(serde_json::json!({ "live_raffles": n })),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
