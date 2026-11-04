//! Public JSON endpoints, and the web app's runtime configuration.

use axum::{
    Extension, Json, Router,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rafi_store::RaffleRow;

use super::{AppState, build_info, pages, service, view};

pub fn routes() -> Router {
    Router::new()
        .route("/api/r/{slug}/view", get(full_view))
        .route("/api/r/{slug}/participant", get(participant))
        .route("/api/r/{slug}/verify.json", get(verify_json))
        .route("/api/challenge", get(challenge))
        .route("/config.js", get(config_js))
        .route("/healthz", get(healthz))
        .route("/.well-known/security.txt", get(security_txt))
}

/// What the web app needs from the server before it starts: `/config.js`
/// sets it as `window.__RAFI_CONFIG__`, loaded by `index.html` ahead of the
/// app's own script, so nothing of an installation's is compiled into the
/// web app bundle.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WebConfig {
    /// Clerk's publishable key when the server verifies Clerk sessions;
    /// `null` under dev auth, when the admin shows that instead of a sign-in.
    clerk_publishable_key: Option<String>,
    /// Which Rafi this is ([`build_info::VERSION`]), for the footers.
    version: &'static str,
    /// Its source ([`build_info::source_url`]), which the footers link.
    source_url: String,
    /// Its independent verifier ([`build_info::verifier_url`]), which the
    /// verification page links.
    verifier_url: String,
    /// The organizers' guide ([`build_info::guide_url`]), which the admin links.
    guide_url: String,
    /// Whether the operator has provided terms (`/terms`), which organizers
    /// accept by using the service: the admin says so and the home page links
    /// them only then.
    terms: bool,
    /// Whether the operator has provided a privacy notice (`/privacy`),
    /// which the home page links only then.
    privacy: bool,
}

/// How long a `security.txt` says it holds: RFC 9116 wants an expiry,
/// less than a year away; it is written fresh on every request.
const SECURITY_TXT_DAYS: i64 = 180;

/// `/.well-known/security.txt` (RFC 9116): whom to tell about a security
/// problem with this installation (its operator, `RAFI_OPERATOR_EMAIL`), and
/// Rafi's policy for vulnerabilities in the software itself.
async fn security_txt(Extension(app): Extension<AppState>) -> Response {
    let expires = rafi_lifecycle::dates::Date::of_utc(app.clock.now() + SECURITY_TXT_DAYS * 86_400);
    let mut body = String::new();
    if !app.cfg.operator.email.is_empty() {
        body.push_str(&format!("Contact: mailto:{}\n", app.cfg.operator.email));
    }
    body.push_str(&format!(
        "Expires: {}T00:00:00Z\nPolicy: {}/security/policy\nPreferred-Languages: en\nCanonical: {}/.well-known/security.txt\n",
        expires.iso(),
        build_info::SOURCE_URL,
        app.cfg.public_url.trim_end_matches('/'),
    ));
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

/// `ok` and the version: up, and which Rafi ([`build_info::VERSION`]).
async fn healthz() -> String {
    format!("ok {}", build_info::VERSION)
}

/// The web app's configuration ([`WebConfig`]), as a script, and the saved
/// theme applied before first paint (`pages::APPLY_THEME`: `index.html` loads
/// this in its head, and its policy lets it run nothing inline). `no-cache`: a
/// rotated key takes effect on the next page load.
async fn config_js(Extension(app): Extension<AppState>) -> Response {
    let cfg = WebConfig {
        clerk_publishable_key: (!app.cfg.dev_auth)
            .then(|| app.cfg.clerk_publishable_key.clone())
            .flatten(),
        version: build_info::VERSION,
        source_url: build_info::source_url(),
        verifier_url: build_info::verifier_url(),
        guide_url: build_info::guide_url(),
        terms: app.cfg.documents.terms.is_some(),
        privacy: app.cfg.documents.privacy.is_some(),
    };
    // A JSON object is a JavaScript object literal; as an external script,
    // nothing in it can close a tag.
    let json = serde_json::to_string(&cfg).expect("a plain struct");
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        format!("window.__RAFI_CONFIG__ = {json};\n{}\n", pages::APPLY_THEME),
    )
        .into_response()
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

/// The full view: what the projector and the admin show. They poll it. Not
/// to be indexed, as the projector isn't ([`crate::noindex`]): it has the
/// recent names.
async fn full_view(Extension(app): Extension<AppState>, Path(slug): Path<String>) -> Response {
    let raffle = match public_raffle(&app, &slug).await {
        Ok(r) => r,
        Err(status) => return status.into_response(),
    };
    match view::build(&app, &raffle.id).await {
        Ok(Some(v)) => {
            let mut resp = ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response();
            crate::noindex(&mut resp);
            resp
        }
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
/// is never served from a cache; a finished raffle costs them a 304. Not to
/// be indexed, as the verification page isn't ([`crate::noindex`]): it has
/// every entrant's name.
async fn verify_json(
    Extension(app): Extension<AppState>,
    Path(slug): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let mut resp = verify_response(&app, &slug, &headers).await;
    crate::noindex(&mut resp);
    resp
}

async fn verify_response(app: &AppState, slug: &str, headers: &axum::http::HeaderMap) -> Response {
    match service::verify_data(app, slug).await {
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
