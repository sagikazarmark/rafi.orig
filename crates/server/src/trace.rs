//! Request IDs: what ties an organizer's "I got an error" to a log line.
//!
//! Every request through [`layer`] gets an ID, in a span around everything
//! it does, so each line logged while it runs carries it, and in the
//! response's `x-request-id` header, which the admin shows with a server
//! error ("reference: ..."). Both hosts add it around Clerk's layer: the
//! native server in [`build_router`](crate::build_router), the Worker in
//! its `app()`.
//!
//! The ID is [`RequestIds`]'s: natively 16 random hex characters; on
//! Workers Cloudflare's Ray ID (`cf-ray`), set by its edge, so the reference
//! is the one Cloudflare's own logs show for the request.
//!
//! The span holds the method and the route's pattern ([`MatchedPath`], so
//! `/j/{code}`, never the raw path: it holds the join code) and nothing
//! else of the request: no query, no address, no user agent, no cookie.
//! When the request is done, one line says how it went, with its status
//! and latency: a warning for a server error (5xx), at debug otherwise
//! (the projector polls every few seconds; that is no log).
//!
//! The span is at `info`, so the ID is on the lines of a native server
//! logging at `info` or more (the default); `RUST_LOG=warn` drops it.
//! On Workers the console layer lets spans through
//! (`apps/worker/src/log.rs`).
//!
//! The latency is the wall clock's ([`rafi_store::now_ms`]), in
//! milliseconds: `std::time::Instant` aborts on `wasm32-unknown-unknown`
//! (which is why this isn't `tower_http::trace`). On Workers the clock only
//! moves across I/O, so there it is the time spent waiting on I/O.

use axum::{
    Router,
    extract::{MatchedPath, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use tracing::Instrument as _;

/// The response header carrying the request's ID.
pub const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// The longest inbound ID taken as is (a Ray ID is about 20 characters).
const MAX_LEN: usize = 64;

/// Where a request's ID comes from.
#[derive(Clone, Debug)]
pub enum RequestIds {
    /// A fresh one: 16 random hex characters (the native server).
    Generate,
    /// The request's header, set by something in front that can be trusted
    /// to set it (`cf-ray` on Workers, [`RequestIds::cloudflare`]); a fresh
    /// one if it is missing or isn't a plain token (at most 64 letters,
    /// digits and dashes), so nothing a client wrote reaches the log.
    Header(HeaderName),
}

impl RequestIds {
    /// Cloudflare's Ray ID: Cloudflare sets `cf-ray` on every request it
    /// passes to a Worker, overwriting a client's.
    pub fn cloudflare() -> Self {
        Self::Header(HeaderName::from_static("cf-ray"))
    }

    /// The ID for a request with `headers`.
    fn pick(&self, headers: &HeaderMap) -> String {
        match self {
            Self::Generate => generate(),
            Self::Header(name) => headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .filter(|v| well_formed(v))
                .map_or_else(generate, str::to_owned),
        }
    }
}

/// 16 random lowercase hex characters (64 bits): short enough to read out
/// or paste, plenty to tell requests apart.
fn generate() -> String {
    hex::encode(crate::ids::random_bytes::<8>())
}

/// A token safe to log and to send back: letters, digits and dashes.
fn well_formed(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_LEN
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `router` with request IDs: a span per request, the `x-request-id`
/// header, a line per request. Added last, so it is around everything else
/// on the router (Clerk's layer included) and runs after routing, where the
/// route's pattern is known.
pub fn layer(router: Router, ids: RequestIds) -> Router {
    router.layer(axum::middleware::from_fn_with_state(ids, trace))
}

async fn trace(State(ids): State<RequestIds>, req: Request, next: Next) -> Response {
    let id = ids.pick(req.headers());
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map_or("fallback", MatchedPath::as_str);
    let span = tracing::info_span!(
        "request",
        id = %id,
        method = %req.method(),
        route = %route,
    );
    let start = rafi_store::now_ms();
    let mut resp = next.run(req).instrument(span.clone()).await;
    let latency_ms = rafi_store::now_ms().saturating_sub(start);
    let status = resp.status().as_u16();
    span.in_scope(|| {
        if resp.status().is_server_error() {
            tracing::warn!(status, latency_ms, "request failed");
        } else {
            tracing::debug!(status, latency_ms, "request");
        }
    });
    if let Ok(value) = HeaderValue::from_str(&id) {
        resp.headers_mut().insert(REQUEST_ID, value);
    }
    resp
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Router, body::Body, http::Request, http::StatusCode, routing::get};
    use tower::ServiceExt as _;
    use tracing_subscriber::fmt::MakeWriter;

    use super::{RequestIds, layer};

    /// What the subscriber wrote.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Captured {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Captured {
        type Writer = Self;
        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }

    /// A router with a route that fails (holding a code, like the join
    /// page's) and one that doesn't, both logging a line of their own.
    fn router(ids: RequestIds) -> Router {
        let routes = Router::new()
            .route(
                "/j/{code}",
                get(|| async {
                    tracing::error!("inside the handler");
                    StatusCode::INTERNAL_SERVER_ERROR
                }),
            )
            .route("/ok", get(|| async { "ok" }));
        layer(routes, ids)
    }

    async fn call(router: &Router, req: Request<Body>) -> (StatusCode, Option<String>) {
        let resp = router.clone().oneshot(req).await.unwrap();
        let id = resp
            .headers()
            .get(super::REQUEST_ID)
            .map(|v| v.to_str().unwrap().to_owned());
        (resp.status(), id)
    }

    fn get_req(path: &str) -> Request<Body> {
        Request::get(path).body(Body::empty()).unwrap()
    }

    fn generated(id: &str) -> bool {
        id.len() == 16
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    #[tokio::test]
    async fn every_response_carries_a_fresh_id() {
        let router = router(RequestIds::Generate);
        let (status, first) = call(&router, get_req("/ok")).await;
        assert_eq!(status, StatusCode::OK);
        let first = first.expect("an x-request-id");
        assert!(generated(&first), "{first}");
        let (_, second) = call(&router, get_req("/ok")).await;
        assert_ne!(Some(first), second, "each request its own");
        // Unrouted requests too.
        let (status, id) = call(&router, get_req("/nowhere")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(id.is_some_and(|id| generated(&id)));
        // An inbound header is no ID unless the deployment says so.
        let mut req = get_req("/ok");
        req.headers_mut()
            .insert("x-request-id", "chosen-by-client".parse().unwrap());
        let (_, id) = call(&router, req).await;
        assert!(id.is_some_and(|id| generated(&id)));
    }

    #[tokio::test]
    async fn the_ray_id_on_workers() {
        let router = router(RequestIds::cloudflare());
        let with_ray = |ray: &str| {
            let mut req = get_req("/ok");
            req.headers_mut().insert("cf-ray", ray.parse().unwrap());
            req
        };
        let (_, id) = call(&router, with_ray("8f1a2b3c4d5e6f70-AMS")).await;
        assert_eq!(id.as_deref(), Some("8f1a2b3c4d5e6f70-AMS"));
        // Not a plain token, or too long: a fresh ID instead.
        for bad in ["a b", "x\"y", "id;evil", &"a".repeat(65), ""] {
            let (_, id) = call(&router, with_ray(bad)).await;
            assert!(id.as_deref().is_some_and(generated), "{bad:?}: {id:?}");
        }
        // Without one (wrangler dev): a fresh ID.
        let (_, id) = call(&router, get_req("/ok")).await;
        assert!(id.as_deref().is_some_and(generated), "{id:?}");
    }

    #[tokio::test]
    async fn the_log_ties_the_id_to_the_failure_without_the_path() {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let router = router(RequestIds::Generate);

        let (status, id) = call(&router, get_req("/j/SECRETCODE")).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let id = id.unwrap();
        let log = captured.text();
        let lines: Vec<&str> = log.lines().collect();
        assert_eq!(lines.len(), 2, "{log}");
        // The handler's own line and the request's, both with the ID, the
        // method and the route's pattern.
        for line in &lines {
            assert!(line.contains(&format!("id={id}")), "{line}");
            assert!(line.contains("method=GET"), "{line}");
            assert!(line.contains("route=/j/{code}"), "{line}");
        }
        assert!(lines[0].contains("ERROR") && lines[0].contains("inside the handler"));
        assert!(lines[1].contains("WARN") && lines[1].contains("request failed"));
        assert!(lines[1].contains("status=500"), "{}", lines[1]);
        assert!(lines[1].contains("latency_ms="), "{}", lines[1]);
        assert!(
            !log.contains("SECRETCODE"),
            "the raw path is never logged: {log}"
        );

        // A success leaves no line at info.
        let (status, _) = call(&router, get_req("/ok")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(captured.text(), log);
    }
}
