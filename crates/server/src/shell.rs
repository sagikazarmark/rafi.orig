//! The web app's shell: its `index.html`, which the server answers every
//! path nothing else does with (the web app routes in the browser), under
//! the web app's Content-Security-Policy (`csp::Page::App`).
//!
//! The server serves it, not the platform's static files, because the policy
//! names the installation's Clerk host, which only the configuration knows.
//! On Workers the shell is compiled into the Worker (it ships with the
//! files it links, in one artifact); natively it is read from the web app's
//! directory at startup (`Dirs::shell`), next to the files it links.

use std::sync::Arc;

use anyhow::Context as _;
use axum::{
    Extension,
    body::Bytes,
    http::{HeaderMap, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
};

use crate::{csp, revalidate};

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
        let text = std::str::from_utf8(&html).context("the web app's index.html is not UTF-8")?;
        if let Some(script) = csp::inline_scripts(text).first() {
            anyhow::bail!(
                "the web app's index.html has an inline script, which its Content-Security-Policy doesn't allow (a script of the server's, like /config.js, does): {:?}",
                script.trim().chars().take(80).collect::<String>()
            );
        }
        let etag = revalidate::etag(&html).into();
        Ok(Self { html, etag })
    }
}

/// Whether `path` is under one of the server's own prefixes (its files, its
/// APIs) rather than a route of the web app's: a miss there is a 404, not
/// the shell.
fn servers_own(path: &str) -> bool {
    ["/assets/", "/participant/", "/api/", "/legal/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// The shell, for a `GET` (routed as one) of any of the web app's paths;
/// revalidated on every load, so a deploy's new shell (linking new names) is
/// picked up on the next. A 304 carries the shell's policy too: a browser
/// keeps a 304's headers in place of the cached response's.
pub(crate) async fn serve(
    Extension(shell): Extension<Shell>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let path = uri.path();
    if servers_own(path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut resp = revalidate::no_cache(&headers, &shell.etag, Html(shell.html));
    resp.extensions_mut().insert(csp::Page::App {
        frameable: crate::frameable(path),
    });
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_with_an_inline_script_is_refused() {
        let err = Shell::new("<head><script>alert(1)</script></head>").unwrap_err();
        assert!(err.to_string().contains("inline script"), "{err}");
        assert!(Shell::new(r#"<head><script src="/config.js"></script></head>"#).is_ok());
    }
}
