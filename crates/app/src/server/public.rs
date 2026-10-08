//! Public JSON and SSE endpoints.

use std::{convert::Infallible, time::Duration};

use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, header},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use futures::StreamExt;
use tokio_stream::wrappers::WatchStream;

use super::{AppState, db, service};

pub fn routes() -> Router {
    Router::new()
        .route("/api/r/{slug}/events", get(events))
        .route("/api/r/{slug}/view", get(view))
        .route("/api/r/{slug}/verify.json", get(verify_json))
        .route("/api/altcha", get(altcha))
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

/// Live view of a raffle as server-sent events. The latest view is sent
/// immediately on connect, so reconnecting clients never need a replay.
async fn events(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    let raffle = match public_raffle(&app, &slug).await {
        Ok(r) => r,
        Err(status) => return status.into_response(),
    };
    let rx = app.hub.subscribe(&raffle.id);
    if rx.borrow().is_none() {
        app.refresh(&raffle.id).await;
    }
    let views = WatchStream::new(rx).filter_map(|view| async move {
        let view = view?;
        let json = serde_json::to_string(&*view).ok()?;
        Some(Ok::<_, Infallible>(
            Event::default().event("view").data(json),
        ))
    });
    // Real events (not SSE comments) so clients can detect stale connections.
    let pings =
        tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(Duration::from_secs(15)))
            .skip(1)
            .map(|_| Ok::<_, Infallible>(Event::default().event("ping").data("")));
    let stream = futures::stream::select(views, pings);
    let mut resp = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response();
    // Tell reverse proxies not to buffer the stream.
    resp.headers_mut()
        .insert("x-accel-buffering", "no".parse().expect("header"));
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, "no-cache".parse().expect("header"));
    resp
}

/// The current view as plain JSON (polling fallback).
async fn view(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    let raffle = match public_raffle(&app, &slug).await {
        Ok(r) => r,
        Err(status) => return status.into_response(),
    };
    if app.hub.current(&raffle.id).is_none() {
        app.refresh(&raffle.id).await;
    }
    match app.hub.current(&raffle.id) {
        Some(v) => ([(header::CACHE_CONTROL, "no-store")], Json(&*v)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
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

async fn altcha(Extension(app): Extension<AppState>) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(app.altcha.challenge()),
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
