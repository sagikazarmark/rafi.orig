//! The Content-Security-Policy: what a page of Rafi's may load and run, so
//! that markup an attacker got past the escaping can't run scripts of its
//! own.
//!
//! A response says what kind of page it is ([`Page`], a response extension)
//! and [`security_headers`](crate::security_headers) sends that kind's policy
//! ([`Policies`], built once from the configuration):
//!
//! - **The server's pages** (`/j/`, `/rules/`, `/privacy/{slug}`, a 404, and
//!   any HTML that doesn't say): scripts from this origin and the inline ones
//!   by their hashes (the head script and the participant build's loader,
//!   `pages::inline_scripts`), WebAssembly, styles and images from
//!   this origin (and `data:`, daisyUI's icons in the stylesheet), the live
//!   channel's socket.
//! - **The installation's documents** (`/privacy`, `/terms`): the same, and
//!   images from any `https:` origin, which the operator's Markdown may link.
//! - **The web app** (its `index.html`, [`crate::shell`]): the same as the
//!   server's pages, without inline scripts (the shell has none: its theme
//!   is applied by `/config.js`), but with `'unsafe-eval'` (Dioxus's
//!   `document::eval`, `document::Title` among its users, runs through
//!   `new Function`) and inline styles (Dioxus sets `style` attributes; Clerk
//!   styles itself with CSS-in-JS), and what Clerk needs: its Frontend API
//!   host, which the publishable key names, and its bot and fraud protection
//!   ([`dioxus_clerk::csp::ClerkCsp`]). None of Clerk's under dev auth, which
//!   loads no Clerk.
//!
//! Every policy denies framing (`frame-ancestors 'none'`), except the web
//! app's at the paths [`crate::frameable`] names. Responses that aren't HTML
//! carry `frame-ancestors 'none'` alone: there is nothing in them to run.
//!
//! `RAFI_CSP=report-only` ([`CspMode`]) sends the policy as
//! `Content-Security-Policy-Report-Only`, framing still denied by an enforced
//! `frame-ancestors`.

use std::sync::LazyLock;

use axum::http::{HeaderName, HeaderValue, header};
use base64::Engine as _;
use dioxus_clerk::csp::ClerkCsp;
use sha2::{Digest, Sha256};

use crate::config::{Config, CspMode};

/// What kind of page a response is, and so which policy it carries: a
/// response extension. HTML without one is a [`Page::Server`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// A page the server renders.
    Server,
    /// The installation's privacy notice or terms.
    Legal,
    /// The web app's shell.
    App,
}

/// `frame-ancestors 'none'`: what every response that may not be framed
/// carries.
pub const NO_FRAMING: &str = "frame-ancestors 'none'";

/// The policies, rendered once from the configuration.
#[derive(Debug, Clone)]
pub struct Policies {
    mode: CspMode,
    server: HeaderValue,
    legal: HeaderValue,
    app: HeaderValue,
    app_framed: HeaderValue,
}

impl Policies {
    /// The policies `cfg` calls for. A publishable key no host decodes from
    /// ([`Config::validate`] refuses one outside dev auth) adds nothing of
    /// Clerk's.
    pub fn new(cfg: &Config) -> Self {
        let live_channel = live_channel(&cfg.public_url);
        let base = Policy::default()
            .with("default-src", ["'self'"])
            .with("script-src", ["'self'", "'wasm-unsafe-eval'"])
            .with("style-src", ["'self'"])
            .with("img-src", ["'self'", "data:"])
            .with("connect-src", ["'self'"])
            .with("connect-src", live_channel)
            .with("worker-src", ["'self'"])
            .with("object-src", ["'none'"])
            .with("base-uri", ["'none'"])
            .with("form-action", ["'self'"]);

        let server = base.clone().with("script-src", SERVER_SCRIPT_HASHES.iter());
        let legal = server.clone().with("img-src", ["https:"]);

        let mut app = base
            .with("script-src", ["'unsafe-eval'"])
            .with("style-src", ["'unsafe-inline'"]);
        if let Some(clerk) = clerk(cfg) {
            for (directive, sources) in clerk.directives() {
                app = app.with(directive, sources);
            }
        }
        let app_framed = app.render();
        let deny = |p: Policy| p.with("frame-ancestors", ["'none'"]).render();
        Self {
            mode: cfg.csp,
            server: value(deny(server)),
            legal: value(deny(legal)),
            app: value(deny(app)),
            app_framed: value(app_framed),
        }
    }

    /// The header the policies go in.
    pub fn header(&self) -> HeaderName {
        match self.mode {
            CspMode::Enforce => header::CONTENT_SECURITY_POLICY,
            CspMode::ReportOnly => header::CONTENT_SECURITY_POLICY_REPORT_ONLY,
        }
    }

    /// Whether the policies are only reported.
    pub fn report_only(&self) -> bool {
        self.mode == CspMode::ReportOnly
    }

    /// The policy of `page`; `framed`: at a path that may be framed, which
    /// only the web app's honours.
    pub fn policy(&self, page: Page, framed: bool) -> &HeaderValue {
        match page {
            Page::Server => &self.server,
            Page::Legal => &self.legal,
            Page::App if framed => &self.app_framed,
            Page::App => &self.app,
        }
    }
}

/// Clerk's share of the web app's policy, unless dev auth is on.
fn clerk(cfg: &Config) -> Option<ClerkCsp> {
    if cfg.dev_auth {
        return None;
    }
    ClerkCsp::from_publishable_key(cfg.clerk_publishable_key.as_deref()?)
}

/// The live channel's origin as a `connect-src` source: `wss://` for an
/// `https://` public URL, `ws://` for `http://`. `'self'` covers a same-origin
/// socket in browsers that implement CSP Level 3, older Safari among the
/// exceptions; phones are why it's named.
fn live_channel(public_url: &str) -> Option<String> {
    let (scheme, rest) = public_url.split_once("://")?;
    let ws = match scheme {
        "https" => "wss",
        "http" => "ws",
        _ => return None,
    };
    let host = rest.split(['/', '?', '#']).next()?;
    let plain = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    plain.then(|| format!("{ws}://{}", host.to_ascii_lowercase()))
}

/// The hash sources of the server pages' inline scripts.
static SERVER_SCRIPT_HASHES: LazyLock<Vec<String>> = LazyLock::new(|| {
    crate::pages::inline_scripts()
        .into_iter()
        .map(hash_source)
        .collect()
});

/// `script` as a hash source: `'sha256-<base64>'`.
pub fn hash_source(script: &str) -> String {
    let digest = Sha256::digest(script.as_bytes());
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

/// The contents of `html`'s inline scripts that a browser runs: every
/// `<script>` without a `src`, but data blocks (`type="application/json"`).
/// For the markup Rafi writes itself, not for arbitrary HTML.
pub fn inline_scripts(html: &str) -> Vec<&str> {
    let lower = html.to_ascii_lowercase();
    let mut scripts = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<script").map(|i| at + i) {
        let Some(open_end) = lower[start..].find('>').map(|i| start + i) else {
            break;
        };
        let Some(close) = lower[open_end..].find("</script").map(|i| open_end + i) else {
            break;
        };
        let attrs = &lower[start + "<script".len()..open_end];
        let external = attrs.contains("src=");
        let data = attrs.contains("application/json");
        if !external && !data {
            scripts.push(&html[open_end + 1..close]);
        }
        at = close;
    }
    scripts
}

/// A policy under construction: directives in the order first added, each
/// with its sources in the order first added, none twice.
#[derive(Debug, Clone, Default)]
struct Policy(Vec<(&'static str, Vec<String>)>);

impl Policy {
    fn with<S: AsRef<str>>(
        mut self,
        directive: &'static str,
        sources: impl IntoIterator<Item = S>,
    ) -> Self {
        let i = match self.0.iter().position(|(d, _)| *d == directive) {
            Some(i) => i,
            None => {
                self.0.push((directive, Vec::new()));
                self.0.len() - 1
            }
        };
        let list = &mut self.0[i].1;
        for source in sources {
            let source = source.as_ref();
            if !list.iter().any(|s| s == source) {
                list.push(source.to_owned());
            }
        }
        self
    }

    fn render(&self) -> String {
        self.0
            .iter()
            .map(|(directive, sources)| format!("{directive} {}", sources.join(" ")))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// A rendered policy as a header value. Every source is a fixed string, a
/// hash, a host `ClerkCsp` checked, or [`live_channel`]'s plain host, so it
/// is always one.
fn value(policy: String) -> HeaderValue {
    HeaderValue::try_from(policy).expect("a policy is a valid header value")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_scripts_are_the_ones_that_run() {
        let html = r#"<head><script>a()</script><script src="/x.js"></script>
            <SCRIPT type="module">b()</SCRIPT><script type="application/json" id="c">{"d":1}</script>
            <script async src="/y.js" ></script><script>
e()
</script></head>"#;
        assert_eq!(inline_scripts(html), ["a()", "b()", "\ne()\n"]);
        assert!(inline_scripts("<p>no scripts</p>").is_empty());
    }

    #[test]
    fn hash_sources_are_base64_sha256() {
        // printf 'alert(1)' | openssl dgst -sha256 -binary | base64
        assert_eq!(
            hash_source("alert(1)"),
            "'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI='"
        );
    }

    #[test]
    fn the_live_channel_is_the_public_origin_as_a_socket() {
        assert_eq!(
            live_channel("https://Raffle.example.org").as_deref(),
            Some("wss://raffle.example.org")
        );
        assert_eq!(
            live_channel("http://127.0.0.1:8787/").as_deref(),
            Some("ws://127.0.0.1:8787")
        );
        assert_eq!(live_channel("raffle.example.org"), None);
        assert_eq!(live_channel("ftp://raffle.example.org"), None);
        assert_eq!(live_channel("https://a b;script-src *"), None);
    }

    #[test]
    fn a_policy_merges_directives_and_sources() {
        let p = Policy::default()
            .with("script-src", ["'self'"])
            .with("img-src", ["data:"])
            .with("script-src", ["'self'", "https://a"]);
        assert_eq!(p.render(), "script-src 'self' https://a; img-src data:");
    }
}
