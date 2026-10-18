//! A challenge endpoint for [axum](https://docs.rs/axum) (the `axum`
//! feature). Redeeming stays in your handler, where your use case decides
//! what goes first (see the [crate] docs).

use axum::{
    Json,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::Verifier;

/// `GET` handler: a fresh challenge as JSON, never cached. With the verifier
/// as state, or reachable from it ([`FromRef`](axum::extract::FromRef)):
///
/// ```
/// use axum::{Router, routing::get};
/// use altcha_server::{Verifier, axum::challenge};
///
/// let verifier = Verifier::new("a long random secret", 5_000).unwrap();
/// let app: Router = Router::new().route("/api/altcha", get(challenge)).with_state(verifier);
/// ```
///
/// A challenge that can't be made is a 500; to log why, call
/// [`challenge_response`] from your own handler.
pub async fn challenge(State(verifier): State<Verifier>) -> Response {
    challenge_response(&verifier)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// A fresh challenge from `verifier` as the response to send: JSON, with
/// `Cache-Control: no-store` (each one is single-use).
pub fn challenge_response(verifier: &Verifier) -> Result<Response, altcha::Error> {
    let challenge = verifier.challenge()?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(challenge)).into_response())
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt as _;

    use super::*;

    #[tokio::test]
    async fn serves_fresh_uncached_challenges() {
        let verifier = Verifier::new("k", 1).unwrap();
        let app: Router = Router::new()
            .route("/api/altcha", get(challenge))
            .with_state(verifier);
        let response = app
            .oneshot(Request::get("/api/altcha").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let c: altcha::Challenge = serde_json::from_slice(&body).unwrap();
        assert_eq!(c.parameters.cost, 1);
        assert!(c.signature.is_some());
    }
}
