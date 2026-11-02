//! What the request log says, in a test binary of its own: the subscriber it
//! installs sees every event of its thread, and tracing's callsite interest is
//! shared by the whole process, so beside other tests (in parallel, with
//! their own subscribers or none) a line could go missing or a stranger's
//! appear. Alone here, the log is exactly the requests'.

use std::sync::{Arc, Mutex};

use axum::{Router, body::Body, http::Request, http::StatusCode, routing::get};
use rafi_server::trace::{REQUEST_ID, RequestIds, layer};
use tower::ServiceExt as _;
use tracing_subscriber::fmt::MakeWriter;

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

/// A router with a route that fails (holding a code, like the join page's)
/// and one that doesn't, both logging a line of their own.
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

async fn call(router: &Router, path: &str) -> (StatusCode, Option<String>) {
    let req = Request::get(path).body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let id = resp
        .headers()
        .get(REQUEST_ID)
        .map(|v| v.to_str().unwrap().to_owned());
    (resp.status(), id)
}

#[tokio::test(flavor = "current_thread")]
async fn the_log_ties_the_id_to_the_failure_without_the_path() {
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let router = router(RequestIds::Generate);

    let (status, id) = call(&router, "/j/SECRETCODE").await;
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
    let (status, _) = call(&router, "/ok").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(captured.text(), log);
}
