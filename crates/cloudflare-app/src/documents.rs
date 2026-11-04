//! The operator's legal documents, from the Worker's static files.
//!
//! `legal/terms.md` and `legal/privacy.md` sit among the static files when
//! the operator provides them (an installation's deploy copies its `legal/`
//! in), and the Worker reads them through the assets binding when an isolate
//! builds its app. Nothing of an installation's is compiled in: one build of
//! the Worker serves every installation, and a deploy with new documents is
//! a deploy of files. Each is optional: one not there is a page saying so.
//! One there names its version in its front matter ([`Document::parse`]);
//! one without refuses every request, as the native server refuses to start.

use http_body_util::BodyExt as _;
use rafi_server::config::{Document, Documents};
use worker::{Env, Fetcher};

/// The assets binding (`[assets] binding` in `wrangler.toml`).
pub const BINDING: &str = "ASSETS";
/// The terms' path among the static files.
pub const TERMS: &str = "/legal/terms.md";
/// The privacy notice's path among the static files.
pub const PRIVACY: &str = "/legal/privacy.md";

/// The documents the static files have, either or both.
pub async fn documents(env: &Env) -> anyhow::Result<Documents> {
    let assets = env.assets(BINDING)?;
    Ok(Documents {
        terms: document(&assets, TERMS).await?,
        privacy: document(&assets, PRIVACY).await?,
    })
}

/// The document at `path`, if the static files have one.
async fn document(assets: &Fetcher, path: &str) -> anyhow::Result<Option<Document>> {
    let Some(source) = text(assets, path).await? else {
        return Ok(None);
    };
    Ok(Some(Document::parse(
        path.trim_start_matches('/'),
        &source,
    )?))
}

/// A text file from the static files, `None` when there is none (a 404:
/// there is no single-page fallback among them, the Worker serves the web
/// app's shell).
async fn text(assets: &Fetcher, path: &str) -> anyhow::Result<Option<String>> {
    // The binding looks the path up; the host is anything.
    let resp = assets.fetch(format!("https://assets{path}"), None).await?;
    if resp.status() != http::StatusCode::OK {
        return Ok(None);
    }
    let bytes = resp.into_body().collect().await?.to_bytes();
    Ok(Some(String::from_utf8(bytes.to_vec())?))
}
