//! The installation's legal documents, from the Worker's static files.
//!
//! `legal/terms.md` and `legal/privacy.md` sit among the static files
//! (`worker-assets/legal/`, where `scripts/app-assets.sh` copies them from
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
            "{TERMS} and {PRIVACY} are missing from the Worker's static files: the installation's terms.md and privacy.md go into its deploy/cloudflare legal/ (npm run build copies them in), or into RAFI_DOCS_DIR for scripts/app-assets.sh"
        ),
    }
}

/// A text file from the static files, `None` when there is none. Assets
/// answers a missing path with the web app's `index.html`
/// (`not_found_handling = "single-page-application"`, with a 200), so HTML
/// counts as missing too.
async fn text(assets: &Fetcher, path: &str) -> anyhow::Result<Option<String>> {
    // The binding looks the path up; the host is anything.
    let resp = assets.fetch(format!("https://assets{path}"), None).await?;
    let html = resp
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("text/html"));
    if resp.status() != http::StatusCode::OK || html {
        return Ok(None);
    }
    let bytes = resp.into_body().collect().await?.to_bytes();
    Ok(Some(String::from_utf8(bytes.to_vec())?))
}
