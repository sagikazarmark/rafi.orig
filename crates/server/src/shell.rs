//! The web app's shell: its `index.html`, which the server answers every
//! path nothing else does with (the web app routes in the browser), under
//! the web app's Content-Security-Policy ([`crate::csp::Page::App`]).
//!
//! The server serves it, not the platform's static files, because the policy
//! names the installation's Clerk host, which only the configuration knows.
//! On Workers the shell is compiled into the Worker (it ships with the
//! files it links, in one artifact); natively it is read from the web app's
//! directory at startup (`Dirs::shell`), next to the files it links.

use std::sync::Arc;

use axum::{
    Extension,
    body::Bytes,
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use crate::{csp, participant::not_modified};

/// The web app's `index.html`.
#[derive(Debug, Clone)]
pub struct Shell {
    html: Bytes,
    etag: Arc<str>,
}

impl Shell {
    /// The shell `html`. Refused with an inline script, which the web app's
    /// policy doesn't allow: the page would load, and its script not run.
    pub fn new(html: impl Into<Bytes>) -> anyhow::Result<Self> {
        let html: Bytes = html.into();
        let text = std::str::from_utf8(&html)
            .map_err(|e| anyhow::anyhow!("the web app's index.html is not UTF-8: {e}"))?;
        if let Some(script) = csp::inline_scripts(text).first() {
            anyhow::bail!(
                "the web app's index.html has an inline script, which its Content-Security-Policy doesn't allow (a script of the server's, like /config.js, does): {:?}",
                script.trim().chars().take(80).collect::<String>()
            );
        }
        let digest = Sha256::digest(&html);
        let etag = format!("\"{}\"", hex::encode(&digest[..8])).into();
        Ok(Self { html, etag })
    }
}

/// Whether `path` is one of the server's files or APIs rather than a route
/// of the web app's: a miss there is a 404, not the shell.
fn not_the_web_apps(path: &str) -> bool {
    ["/assets/", "/participant/", "/api/", "/legal/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// The shell, for a `GET` of any of the web app's paths; `no-cache` with an
/// entity tag, so a deploy's new shell (linking new names) is picked up on
/// the next load.
pub(crate) async fn serve(
    Extension(shell): Extension<Shell>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if method != Method::GET && method != Method::HEAD {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, "GET, HEAD")],
        )
            .into_response();
    }
    if not_the_web_apps(uri.path()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let cache = [
        (header::ETAG, shell.etag.to_string()),
        (header::CACHE_CONTROL, "no-cache".to_owned()),
    ];
    if not_modified(&headers, &shell.etag) {
        return (StatusCode::NOT_MODIFIED, cache).into_response();
    }
    (cache, Extension(csp::Page::App), Html(shell.html)).into_response()
}
