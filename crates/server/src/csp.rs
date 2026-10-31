//! The Content-Security-Policy: what a page of Rafi's may load and run, so
//! that markup an attacker got past the escaping can't run scripts of its
//! own.
//!
//! A response says what kind of page it is (`Page`, a response extension)
//! and `security_headers` sends that kind's policy (`Policies`, rendered
//! once from the configuration):
//!
//! - **The server's pages** (`/j/`, `/rules/`, `/privacy/{slug}`, a 404, and
//!   any HTML that doesn't say): scripts from this origin and the inline ones
//!   by their hashes (the head script and the participant build's loader,
//!   `pages::embedded_scripts`), WebAssembly, styles and images from this
//!   origin (and `data:`, daisyUI's icons in the stylesheet), the live
//!   channel's socket.
//! - **The installation's documents** (`/privacy`, `/terms`): the same, and
//!   images from any `https:` origin, which the operator's Markdown may link.
//! - **The web app** (its `index.html`, `crate::shell`): the same as the
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
//! app's at the paths `crate::frameable` names. Responses that aren't pages
//! carry `frame-ancestors 'none'` alone: there is nothing in them to run.
//!
//! `RAFI_CSP=report-only` ([`CspMode`]) sends the policy as
//! `Content-Security-Policy-Report-Only`, framing still denied by an enforced
//! `frame-ancestors`.

use std::sync::LazyLock;

use anyhow::Context as _;
use axum::{
    http::{HeaderName, HeaderValue, header},
    response::Response,
};
use base64::Engine as _;
use dioxus_clerk::csp::ClerkCsp;
use sha2::{Digest, Sha256};

use crate::config::Config;

/// Whether the Content-Security-Policy is enforced or only reported
/// (`RAFI_CSP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CspMode {
    /// `Content-Security-Policy`: what the policy doesn't allow doesn't run.
    Enforce,
    /// `Content-Security-Policy-Report-Only`: the browser runs it and says so
    /// in its console.
    ReportOnly,
}

impl CspMode {
    /// `enforce` or `report-only`, in any case.
    pub(crate) fn parse(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "enforce" => Some(Self::Enforce),
            "report-only" => Some(Self::ReportOnly),
            _ => None,
        }
    }

    /// The header a policy goes out in.
    fn header(self) -> HeaderName {
        match self {
            Self::Enforce => header::CONTENT_SECURITY_POLICY,
            Self::ReportOnly => header::CONTENT_SECURITY_POLICY_REPORT_ONLY,
        }
    }
}

/// What kind of page a response is, and so which policy it carries: a
/// response extension. HTML without one is a [`Page::Server`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    /// A page the server renders.
    Server,
    /// The installation's privacy notice or terms.
    Legal,
    /// The web app's shell; `frameable` at the paths `crate::frameable`
    /// names.
    App { frameable: bool },
}

impl Page {
    /// The kind of page `resp` is, if it is one.
    fn of(resp: &Response) -> Option<Self> {
        let html = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/html"));
        resp.extensions()
            .get::<Self>()
            .copied()
            .or(html.then_some(Self::Server))
    }

    fn frameable(self) -> bool {
        matches!(self, Self::App { frameable: true })
    }
}

/// `frame-ancestors 'none'`: what a response that may not be framed carries.
pub(crate) const NO_FRAMING: &str = "frame-ancestors 'none'";

/// The policies, rendered once from the configuration.
#[derive(Debug, Clone)]
pub(crate) struct Policies {
    mode: CspMode,
    server: HeaderValue,
    legal: HeaderValue,
    app: HeaderValue,
    app_frameable: HeaderValue,
}

impl Policies {
    /// The policies `cfg` calls for. A publishable key no host decodes from
    /// ([`Config::validate`] refuses one outside dev auth) adds nothing of
    /// Clerk's.
    pub(crate) fn new(cfg: &Config) -> anyhow::Result<Self> {
        let base = Policy::default()
            .with("default-src", ["'self'"])
            .with("script-src", ["'self'", "'wasm-unsafe-eval'"])
            .with("style-src", ["'self'"])
            .with("img-src", ["'self'", "data:"])
            .with("connect-src", ["'self'"])
            .with("connect-src", live_channel(&cfg.public_url))
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

        let deny = |p: Policy| p.with("frame-ancestors", ["'none'"]);
        Ok(Self {
            mode: cfg.csp,
            server: deny(server).value()?,
            legal: deny(legal).value()?,
            app_frameable: app.value()?,
            app: deny(app).value()?,
        })
    }

    /// The header the policies go out in.
    fn header(&self) -> HeaderName {
        self.mode.header()
    }

    /// Whether the policies are only reported.
    fn report_only(&self) -> bool {
        self.mode == CspMode::ReportOnly
    }

    /// The policy of `page`.
    fn of(&self, page: Page) -> &HeaderValue {
        match page {
            Page::Server => &self.server,
            Page::Legal => &self.legal,
            Page::App { frameable: true } => &self.app_frameable,
            Page::App { frameable: false } => &self.app,
        }
    }

    /// Sets the security headers of `resp` that depend on what it is: its
    /// policy if it is a page, and whether it may be framed.
    pub(crate) fn apply(&self, resp: &mut Response) {
        let page = Page::of(resp);
        let headers = resp.headers_mut();
        // An enforced policy denies framing itself; a reported one doesn't.
        let policy_denies_framing = match page {
            Some(page) => {
                headers.insert(self.header(), self.of(page).clone());
                if page.frameable() {
                    return;
                }
                !self.report_only()
            }
            None => false,
        };
        if !policy_denies_framing {
            headers.insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(NO_FRAMING),
            );
        }
        // For browsers without `frame-ancestors`.
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
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
    crate::pages::embedded_scripts()
        .into_iter()
        .map(hash_source)
        .collect()
});

/// `script` as a hash source: `'sha256-<base64>'` of its text as a browser
/// hashes it, after HTML parsing has made every line break a line feed.
pub(crate) fn hash_source(script: &str) -> String {
    let text = script.replace("\r\n", "\n").replace('\r', "\n");
    let digest = Sha256::digest(text.as_bytes());
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

/// The contents of `html`'s inline scripts that a browser runs: every
/// `<script>` without a `src` attribute, but data blocks
/// (`type="application/json"`).
///
/// For the markup Rafi writes or builds itself, not for arbitrary HTML: an
/// attribute value with a `>` in it, say, would end the tag early. Its
/// mistakes leave a script out, which the policy then refuses: they fail
/// closed.
pub(crate) fn inline_scripts(html: &str) -> Vec<&str> {
    let lower = html.to_ascii_lowercase();
    let mut scripts = Vec::new();
    let mut at = 0;
    while let Some(start) = lower[at..].find("<script").map(|i| at + i) {
        let name_end = start + "<script".len();
        // `<scripts>` or `<script-x>` is another element.
        if !lower[name_end..].starts_with(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        {
            at = name_end;
            continue;
        }
        let Some(open_end) = lower[name_end..].find('>').map(|i| name_end + i) else {
            break;
        };
        let Some(close) = lower[open_end..].find("</script").map(|i| open_end + i) else {
            break;
        };
        let attrs = &lower[name_end..open_end];
        if !has_attribute(attrs, "src") && !attrs.contains("application/json") {
            scripts.push(&html[open_end + 1..close]);
        }
        at = close;
    }
    scripts
}

/// Whether the (lowercase) attributes `attrs` of a tag have `name`: the word
/// itself, not the end of another (`data-src`).
fn has_attribute(attrs: &str, name: &str) -> bool {
    attrs.match_indices(name).any(|(i, _)| {
        let before = attrs[..i].chars().next_back();
        let after = attrs[i + name.len()..].trim_start().chars().next();
        before.is_none_or(|c| c.is_ascii_whitespace() || c == '/')
            && after.is_none_or(|c| c == '=' || c == '/' || c.is_ascii_whitespace())
    })
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

    /// The policy as a header value. Its sources are fixed strings, hashes,
    /// a host `ClerkCsp` checked and [`live_channel`]'s plain host, so this
    /// fails only on a bug.
    fn value(&self) -> anyhow::Result<HeaderValue> {
        let policy = self.render();
        HeaderValue::try_from(&policy)
            .with_context(|| format!("the Content-Security-Policy isn't a header value: {policy}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_scripts_are_the_ones_that_run() {
        let html = r#"<head><script>a()</script><script src="/x.js"></script>
            <SCRIPT type="module">b()</SCRIPT><script type="application/json" id="c">{"d":1}</script>
            <script async src = "/y.js" ></script><script data-src="/z.js">e()</script>
            <scripts>not one</scripts><script>
f()
</script></head>"#;
        assert_eq!(inline_scripts(html), ["a()", "b()", "e()", "\nf()\n"]);
        assert!(inline_scripts("<p>no scripts</p>").is_empty());
    }

    #[test]
    fn the_participant_loader_is_one_inline_script() {
        assert_eq!(inline_scripts(crate::pages::LOADER).len(), 1);
    }

    #[test]
    fn hash_sources_are_base64_sha256_of_the_parsed_text() {
        // printf 'alert(1)' | openssl dgst -sha256 -binary | base64
        assert_eq!(
            hash_source("alert(1)"),
            "'sha256-bhHHL3z2vDgxUt0W3dWQOrprscmda2Y5pLsLg4GF+pI='"
        );
        // A browser's parser makes every line break a line feed first.
        assert_eq!(hash_source("a()\r\nb()\rc()"), hash_source("a()\nb()\nc()"));
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

    #[test]
    fn the_mode_is_enforce_or_report_only() {
        assert_eq!(CspMode::parse("enforce"), Some(CspMode::Enforce));
        assert_eq!(CspMode::parse(" Report-Only "), Some(CspMode::ReportOnly));
        assert_eq!(CspMode::parse("off"), None);
    }
}
