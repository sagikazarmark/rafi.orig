//! The installation's legal documents, from the Worker's static files.
//!
//! `legal/terms.md` and `legal/privacy.md` sit among the static files
//! (`worker-assets/legal/`, where `just assets` in `apps/worker` copies them from
//! `RAFI_DOCS_DIR`), and the Worker reads them through the assets binding
//! when an isolate builds its app, the way the native server reads
//! `RAFI_DOCS_DIR`. Nothing of an installation's is compiled in: one build
//! of the Worker serves every installation, and a deploy with new documents
//! is a deploy of files.

use http_body_util::BodyExt as _;
use rafi_server::config::Documents;
use worker::{Env, Fetcher};

/// The assets binding (`[assets] binding` in `wrangler.toml`).
pub const BINDING: &str = "ASSETS";
/// The terms' path among the static files.
pub const TERMS: &str = "/legal/terms.md";
/// The privacy notice's path among the static files.
pub const PRIVACY: &str = "/legal/privacy.md";

/// The documents, from the static files. Missing (an installation that
/// hasn't supplied them), they are the placeholders under dev auth, which
/// a production configuration refuses anyway, and a refusal here with a
/// message that says what to do.
pub async fn documents(env: &Env, dev_auth: bool) -> anyhow::Result<Documents> {
    let assets = env.assets(BINDING)?;
    match (text(&assets, TERMS).await?, text(&assets, PRIVACY).await?) {
        (Some(terms), Some(privacy)) => Ok(Documents::new(terms, privacy)),
        _ if dev_auth => Ok(Documents::placeholders()),
        _ => anyhow::bail!(
            "{TERMS} and {PRIVACY} are missing from the Worker's static files: the installation's terms.md and privacy.md go into its deploy/cloudflare legal/ (deploy.sh copies them in), or into RAFI_DOCS_DIR for `just assets` in apps/worker"
        ),
    }
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
