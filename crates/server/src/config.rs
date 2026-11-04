//! Runtime configuration from environment variables, and the deployment's
//! legal documents.

#[cfg(not(target_arch = "wasm32"))]
use std::env;

use anyhow::{anyhow, bail};
use rafi_app::RetentionPolicy;

pub use crate::csp::CspMode;
use rafi_raffle::drand::{ChainHash, chains};

/// How the app reaches the `rafi-restate` service: through the Restate ingress.
pub use rafi_engine_restate::ClientConfig as RestateConfig;

/// Who runs this installation: named as the processor in every raffle's
/// privacy notice, and the contact for the installation's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operator {
    pub name: String,
    pub email: String,
}

/// One of the installation's legal documents: its Markdown, and the version
/// its front matter gives it.
///
/// A document starts with front matter naming its version, and nothing
/// else:
///
/// ```text
/// ---
/// version: 2026-11
/// ---
/// # Terms of service
/// ```
///
/// The first line is exactly `---` (a byte order mark before it and
/// whitespace after it aside), then `key: value` lines (blank ones skipped)
/// up to the next `---` line; keys and values are trimmed, and a value in
/// matching quotes loses them. `version` is the only key: required, not
/// empty, given once; any other key is refused, so a typo can't pass for a
/// version-less document. CRLF line endings are read alike. The front matter
/// is not part of [`text`](Self::text), which is what the page renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// What the pages show with the document (`/terms`, `/privacy`) and every
    /// raffle's notice names: a changed document is a new version.
    pub version: String,
    /// The Markdown, without its front matter.
    pub text: String,
}

impl Document {
    /// `source`, the file `name` (named in the error): its front matter read
    /// and stripped. A document without one, or without a `version`, is an
    /// error, which the server refuses to start (a Worker: to serve) with.
    pub fn parse(name: &str, source: &str) -> anyhow::Result<Self> {
        let refuse = |why: &str| {
            anyhow!("{name}: {why}: {FRONT_MATTER} (a line ---, then version: 2026-11, then ---)")
        };
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut lines = source.split_inclusive('\n');
        let mut offset = 0;
        let mut number = 0;
        let mut next = || {
            lines.next().map(|line| {
                offset += line.len();
                number += 1;
                (number, line.trim_end())
            })
        };
        if next().map(|(_, line)| line) != Some("---") {
            return Err(refuse("no front matter"));
        }
        let mut version = None;
        loop {
            let Some((number, line)) = next() else {
                return Err(refuse("its front matter has no closing ---"));
            };
            if line == "---" {
                break;
            }
            if line.trim().is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                return Err(refuse(&format!(
                    "line {number} of its front matter is not key: value"
                )));
            };
            match key.trim() {
                "version" if version.is_some() => {
                    return Err(refuse("its front matter gives the version twice"));
                }
                "version" => version = Some(unquote(value.trim()).trim().to_owned()),
                other => {
                    return Err(refuse(&format!(
                        "its front matter has {other:?}, which isn't a key (only version is)"
                    )));
                }
            }
        }
        let version = match version {
            None => return Err(refuse("its front matter has no version")),
            Some(v) if v.is_empty() => return Err(refuse("its front matter's version is empty")),
            Some(v) => v,
        };
        Ok(Self {
            version,
            text: source[offset..].to_owned(),
        })
    }
}

/// What every refusal of a document's front matter says, and
/// [`refusal`] shows in public: it names a file, never a setting's value.
const FRONT_MATTER: &str = "a legal document starts with front matter naming its version";

/// `value` without one pair of matching quotes around it.
fn unquote(value: &str) -> &str {
    ['"', '\'']
        .into_iter()
        .find_map(|q| value.strip_prefix(q)?.strip_suffix(q))
        .unwrap_or(value)
}

/// The operator's legal documents, each optional: its terms (with the data
/// processing agreement, which organizers accept by using the service) and
/// its privacy notice, as Markdown, each with the version its front matter
/// gives it ([`Document`]). The operator writes them; Rafi ships none. One
/// not provided is `None`: its page says the operator hasn't provided it,
/// and nothing claims what it would have said. Natively they are `terms.md`
/// and `privacy.md` in `RAFI_DOCS_DIR`; on Workers, `legal/terms.md` and
/// `legal/privacy.md` among the Worker's static files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Documents {
    pub terms: Option<Document>,
    pub privacy: Option<Document>,
}

impl Documents {
    /// `terms.md` and `privacy.md` from `dir`, either or both missing, each
    /// one present with its front matter. The directory itself must exist:
    /// `RAFI_DOCS_DIR` pointing nowhere is a mistake, not a choice.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_dir(dir: &std::path::Path) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        if !dir.is_dir() {
            bail!("RAFI_DOCS_DIR {} is not a directory", dir.display());
        }
        let read = |name: &str| -> anyhow::Result<Option<Document>> {
            let path = dir.join(name);
            if !path.exists() {
                return Ok(None);
            }
            let source = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            Document::parse(&path.display().to_string(), &source).map(Some)
        };
        Ok(Self {
            terms: read("terms.md")?,
            privacy: read("privacy.md")?,
        })
    }
}

/// Where the installation runs, which every raffle's privacy notice names
/// as the hosting: the one provider that holds entrants' data that the
/// software alone doesn't determine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Hosting {
    /// The native server, on whatever hosting its operator chose.
    #[default]
    Native,
    /// Cloudflare's Workers, D1 and Durable Objects.
    Cloudflare,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    /// Externally visible base URL, used for QR codes and links.
    pub public_url: String,
    /// Skip Clerk and treat every admin request as user `dev`. Never use in production.
    pub dev_auth: bool,
    /// Clerk's publishable key (`CLERK_PUBLISHABLE_KEY`): what the web app
    /// signs organizers in with. The server hands it to the web app at
    /// runtime (`/config.js`), so one web app bundle serves every
    /// installation; the server's own check is `CLERK_SECRET_KEY`, the hosts'.
    /// Not a secret, but an installation's: required unless dev auth is on,
    /// and a key Clerk's Frontend API host decodes from, which the pages'
    /// Content-Security-Policy names ([`crate::csp`]).
    pub clerk_publishable_key: Option<String>,
    /// Whether the Content-Security-Policy is enforced (`RAFI_CSP=enforce`,
    /// the default) or only reported (`report-only`): the escape hatch for an
    /// installation whose sign-in a policy breaks, say when Clerk starts
    /// loading from a host this build doesn't know yet. Framing stays denied
    /// either way.
    pub csp: CspMode,
    /// The raffle engine: the `Raffle` virtual object of the `rafi-restate`
    /// service. Without an ingress URL, draws can't start.
    pub restate: RestateConfig,
    /// The drand chain new raffles commit to (`RAFI_DRAND_CHAIN`, a chain
    /// hash; quicknet when unset). The app decides it ([`Config::chain`]); the
    /// raffle engine only checks it. `mock` (or `RAFI_DRAND_MOCK=1`) is the mock
    /// chain, in development builds only.
    pub drand_chain: String,
    /// The proof-of-work's HMAC key (`RAFI_POW_KEY`). Random when unset,
    /// so challenges die with the process.
    pub pow_key: String,
    /// The proof-of-work's difficulty (`RAFI_POW_COST`): PBKDF2 iterations
    /// per attempt, about 256 attempts per solve.
    pub pow_cost: u32,
    pub cookie_secure: bool,
    /// The most entrants a raffle takes (`RAFI_MAX_ENTRANTS`): the hard limit
    /// behind the soft rate limits.
    pub max_entrants: i64,
    /// When personal data is erased (`RAFI_RETENTION_DAYS`,
    /// `RAFI_RETENTION_MAX_DAYS`, `RAFI_RETENTION_WINNERS_DAYS`).
    pub retention: RetentionPolicy,
    /// The youngest an entrant may be (`RAFI_MIN_AGE`): rendered in the rules
    /// and on the join form; entrants confirm it by entering.
    pub min_age: u32,
    /// Who runs the installation (`RAFI_OPERATOR_NAME`, `RAFI_OPERATOR_EMAIL`).
    pub operator: Operator,
    /// The operator's legal documents, each optional, with their versions.
    pub documents: Documents,
    /// Where it runs: natively unless the runtime says otherwise (the
    /// Cloudflare app does).
    pub hosting: Hosting,
    /// Where `terms.md` and `privacy.md` are, natively (`RAFI_DOCS_DIR`).
    pub docs_dir: Option<String>,
    /// Numeric variables that were set but didn't parse (`RAFI_RETENTION_DAYS=7d`),
    /// and a `RAFI_CSP` that is neither mode:
    /// [`validate`](Self::validate) refuses them rather than run on a default.
    pub misparsed: Vec<String>,
}

impl Config {
    /// From the process environment (the native server), the legal documents
    /// read from `RAFI_DOCS_DIR`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_env() -> anyhow::Result<Self> {
        let mut cfg = Self::from_lookup(|name| env::var(name).ok());
        if let Some(dir) = &cfg.docs_dir {
            cfg.documents = Documents::from_dir(std::path::Path::new(dir))?;
        }
        Ok(cfg)
    }

    /// From any lookup: the process environment natively, the Worker's vars
    /// and secrets on Workers. Empty values count as unset.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let var = |name: &str| lookup(name).filter(|v| !v.trim().is_empty());
        let flag = |name: &str| {
            var(name).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        };
        let misparsed = std::cell::RefCell::new(Vec::new());
        let int = |name: &str, default: i64| match var(name) {
            None => default,
            Some(v) => v.trim().parse().unwrap_or_else(|_| {
                misparsed.borrow_mut().push(format!("{name}={v:?}"));
                default
            }),
        };
        let dev_auth = flag("RAFI_DEV_AUTH");
        let public_url = var("RAFI_PUBLIC_URL")
            .unwrap_or_else(|| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_owned();
        // Hex: the key is a string, and raw random bytes aren't UTF-8.
        let pow_key =
            var("RAFI_POW_KEY").unwrap_or_else(|| hex::encode(crate::ids::random_bytes::<32>()));
        let csp = match var("RAFI_CSP") {
            None => CspMode::Enforce,
            Some(v) => CspMode::parse(&v).unwrap_or_else(|| {
                misparsed
                    .borrow_mut()
                    .push(format!("RAFI_CSP={v:?} (enforce or report-only)"));
                CspMode::Enforce
            }),
        };
        Self {
            csp,
            database_url: var("RAFI_DATABASE_URL").unwrap_or_else(|| "sqlite://rafi.db".into()),
            cookie_secure: var("RAFI_COOKIE_SECURE")
                .map(|v| v == "1" || v == "true")
                .unwrap_or_else(|| public_url.starts_with("https://")),
            public_url,
            dev_auth,
            clerk_publishable_key: var("CLERK_PUBLISHABLE_KEY"),
            restate: RestateConfig {
                ingress_url: var("RESTATE_INGRESS_URL")
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .to_owned(),
                api_key: var("RESTATE_INGRESS_API_KEY").unwrap_or_default(),
                timeout_secs: u64::try_from(int(
                    "RAFI_ENGINE_TIMEOUT_SECS",
                    i64::try_from(RestateConfig::DEFAULT_TIMEOUT_SECS).unwrap_or(90),
                ))
                .unwrap_or(RestateConfig::DEFAULT_TIMEOUT_SECS),
            },
            // Only the chain matters here: the raffle engine fetches the beacons.
            drand_chain: var("RAFI_DRAND_CHAIN")
                .or_else(|| flag("RAFI_DRAND_MOCK").then(|| MOCK.to_owned()))
                .unwrap_or_else(|| chains::QUICKNET_HASH.to_string()),
            pow_key,
            pow_cost: u32::try_from(int("RAFI_POW_COST", i64::from(DEFAULT_POW_COST)))
                .unwrap_or(DEFAULT_POW_COST),
            max_entrants: int("RAFI_MAX_ENTRANTS", 2000),
            retention: RetentionPolicy {
                days: int("RAFI_RETENTION_DAYS", DEFAULT_RETENTION_DAYS),
                max_days: int("RAFI_RETENTION_MAX_DAYS", DEFAULT_RETENTION_DAYS_MAX),
                winner_hold_days: int("RAFI_RETENTION_WINNERS_DAYS", DEFAULT_WINNER_HOLD_DAYS),
            },
            min_age: u32::try_from(int("RAFI_MIN_AGE", i64::from(DEFAULT_MIN_AGE)))
                .unwrap_or(DEFAULT_MIN_AGE),
            operator: Operator {
                name: var("RAFI_OPERATOR_NAME").unwrap_or_else(|| {
                    if dev_auth {
                        "Rafi (development)".into()
                    } else {
                        String::new()
                    }
                }),
                email: var("RAFI_OPERATOR_EMAIL").unwrap_or_else(|| {
                    if dev_auth {
                        "privacy@example.invalid".into()
                    } else {
                        String::new()
                    }
                }),
            },
            documents: Documents::default(),
            hosting: Hosting::Native,
            docs_dir: var("RAFI_DOCS_DIR"),
            misparsed: misparsed.into_inner(),
        }
    }

    /// Whether the settings make sense together, and a production server
    /// (no dev auth) has everything it must: an operator, Clerk's key and a
    /// long enough proof-of-work key. The documents are optional (a provided
    /// one's version is its own: [`Document::parse`] refuses one without).
    pub fn validate(&self) -> anyhow::Result<()> {
        if !self.misparsed.is_empty() {
            bail!("not a valid value: {}", self.misparsed.join(", "));
        }
        self.retention
            .validate()
            .map_err(|e| anyhow!("RAFI_RETENTION_*: {e}"))?;
        if self.min_age > MAX_MIN_AGE {
            bail!("RAFI_MIN_AGE {} is over {MAX_MIN_AGE}", self.min_age);
        }
        if self.dev_auth {
            return Ok(());
        }
        // The challenges are public and HMAC-signed with this key: a short
        // one is crackable offline, and a forged proof skips the work.
        if self.pow_key.len() < MIN_POW_KEY_CHARS {
            bail!(
                "RAFI_POW_KEY must be at least {MIN_POW_KEY_CHARS} characters (openssl rand -hex 32)"
            );
        }
        if self.operator.name.is_empty() || self.operator.email.is_empty() {
            bail!(
                "RAFI_OPERATOR_NAME and RAFI_OPERATOR_EMAIL are required: the privacy notices name the operator"
            );
        }
        // Without it the web app has no sign-in to show, while every admin
        // call is refused: an admin nobody can enter.
        let Some(key) = &self.clerk_publishable_key else {
            bail!("CLERK_PUBLISHABLE_KEY is required: the web app signs organizers in with it");
        };
        // Clerk loads from, and talks to, the Frontend API host the key
        // names; the policy allows that host, so a key it can't be read
        // from is one Clerk couldn't sign anyone in with either.
        if dioxus_clerk::csp::ClerkCsp::from_publishable_key(key).is_none() {
            bail!(
                "CLERK_PUBLISHABLE_KEY is not a Clerk publishable key (pk_live_… or pk_test_…, from the Clerk dashboard's API keys)"
            );
        }
        Ok(())
    }

    /// The chain new raffles commit to. Release builds commit only to public
    /// drand chains; the mock chain (whose key is public: its beacons can be
    /// forged) needs a development build (`mock-drand`).
    pub fn chain(&self) -> anyhow::Result<ChainHash> {
        resolve_chain(&self.drand_chain, mock_chain())
    }

    /// Configuration for tests: in-memory database, mock drand, no Restate
    /// (the tests hand the app their engine).
    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            database_url: "sqlite::memory:".into(),
            public_url: "http://localhost:8080".into(),
            dev_auth: false,
            clerk_publishable_key: Some(TEST_CLERK_PUBLISHABLE_KEY.into()),
            csp: CspMode::Enforce,
            restate: RestateConfig {
                ingress_url: String::new(),
                api_key: String::new(),
                timeout_secs: RestateConfig::DEFAULT_TIMEOUT_SECS,
            },
            drand_chain: MOCK.into(),
            pow_key: TEST_POW_KEY.into(),
            pow_cost: 1,
            cookie_secure: false,
            max_entrants: 2000,
            retention: RetentionPolicy {
                days: DEFAULT_RETENTION_DAYS,
                max_days: DEFAULT_RETENTION_DAYS_MAX,
                winner_hold_days: DEFAULT_WINNER_HOLD_DAYS,
            },
            min_age: DEFAULT_MIN_AGE,
            operator: Operator {
                name: "Rafi Test Operator".into(),
                email: "privacy@rafi.test".into(),
            },
            documents: Documents {
                terms: Some(Document {
                    version: "test-1".into(),
                    text: "# Test terms\n\nThe test terms, with the **processing agreement**.\n"
                        .into(),
                }),
                privacy: Some(Document {
                    version: "test-1".into(),
                    text: "# Test privacy notice\n\nThe test installation's notice.\n".into(),
                }),
            },
            hosting: Hosting::Native,
            docs_dir: None,
            misparsed: Vec::new(),
        }
    }
}

/// `RAFI_POW_COST` when unset. Provisional: it should be tuned against
/// solve times on slow phones.
pub const DEFAULT_POW_COST: u32 = 1000;
/// The shortest `RAFI_POW_KEY` a production server accepts. The key is
/// the HMAC secret behind every proof-of-work challenge, which is public.
pub const MIN_POW_KEY_CHARS: usize = 32;
/// The tests' key: long enough for a production server.
pub const TEST_POW_KEY: &str = "test-key-test-key-test-key-test-key";
/// The tests' Clerk publishable key: a development instance's, its Frontend
/// API host `rafi-test.clerk.accounts.dev`.
#[cfg(test)]
pub const TEST_CLERK_PUBLISHABLE_KEY: &str = "pk_test_cmFmaS10ZXN0LmNsZXJrLmFjY291bnRzLmRldiQ";
/// `RAFI_RETENTION_DAYS` when unset: a week after the event.
pub const DEFAULT_RETENTION_DAYS: i64 = 7;
/// `RAFI_RETENTION_MAX_DAYS` when unset.
pub const DEFAULT_RETENTION_DAYS_MAX: i64 = 30;
/// `RAFI_RETENTION_WINNERS_DAYS` when unset: how long a winner's data is held
/// past the erasure date unless their prize is marked delivered.
pub const DEFAULT_WINNER_HOLD_DAYS: i64 = 90;
/// `RAFI_MIN_AGE` when unset: the highest age of digital consent in the EU.
pub const DEFAULT_MIN_AGE: u32 = 16;
/// The oldest `RAFI_MIN_AGE` may ask for.
const MAX_MIN_AGE: u32 = 30;

/// What a server that refuses to start (a Worker: every request) says in
/// public about why: `misconfigured`, the reason when it names only
/// settings or files (a setting not set or required, a document without its
/// front matter), never a value, and where the whole reason is. The
/// operator sees it on the first request after a deploy, rather than only in
/// `wrangler tail`.
pub fn refusal(e: &anyhow::Error, log: &str) -> String {
    let reason = e.root_cause().to_string();
    let names_only = reason.ends_with(" is not set")
        || reason.contains(" is not set:")
        || reason.contains(" required")
        || reason.contains("must be at least")
        || reason.contains("has no `dev` feature")
        || reason.contains(FRONT_MATTER);
    if names_only {
        format!("misconfigured: {reason}\n\nThe server's log has the details ({log}).\n")
    } else {
        format!("misconfigured\n\nThe reason is in the server's log ({log}).\n")
    }
}

/// The alias for the mock chain in `RAFI_DRAND_CHAIN`.
const MOCK: &str = "mock";

/// The mock chain's hash, in builds that may commit to it.
fn mock_chain() -> Option<ChainHash> {
    #[cfg(any(test, feature = "mock-drand"))]
    return Some(rafi_raffle::drand::mock_chain().chain().hash());
    #[cfg(not(any(test, feature = "mock-drand")))]
    None
}

/// The chain `configured` names, by the policy of a build whose mock chain is
/// `mock` (`None`: no mock chain allowed).
fn resolve_chain(configured: &str, mock: Option<ChainHash>) -> anyhow::Result<ChainHash> {
    let configured = configured.trim();
    if configured.eq_ignore_ascii_case(MOCK) {
        return mock.ok_or_else(|| {
            anyhow!(
                "RAFI_DRAND_CHAIN is the mock chain, but this build has no `mock-drand` feature"
            )
        });
    }
    let chain: ChainHash = configured
        .parse()
        .map_err(|e| anyhow!("RAFI_DRAND_CHAIN {configured:?}: {e}"))?;
    if chains::by_hash(&chain).is_some() || Some(chain) == mock {
        return Ok(chain);
    }
    bail!("RAFI_DRAND_CHAIN {chain} is not a public drand chain this build knows");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal names a missing setting or a document without its front
    /// matter, never a value; anything else only points at the log.
    #[test]
    fn a_refusal_says_why_without_values() {
        let log = "wrangler tail";
        let body = refusal(&anyhow!("RAFI_POW_KEY is not set"), log);
        assert!(
            body.starts_with("misconfigured: RAFI_POW_KEY is not set") && body.contains(log),
            "{body}"
        );
        // A document without its version: the file named, in public too.
        let body = refusal(
            &Document::parse("legal/terms.md", "# T\n").unwrap_err(),
            log,
        );
        assert!(
            body.starts_with("misconfigured: legal/terms.md: no front matter"),
            "{body}"
        );
        let secret = anyhow!("Clerk configuration: sk_live_secret").context("building the app");
        let body = refusal(&secret, log);
        assert!(
            body.starts_with("misconfigured\n") && !body.contains("sk_live"),
            "{body}"
        );
        let body = refusal(&anyhow!("not a valid value: RAFI_MIN_AGE=\"x\""), log);
        assert!(
            !body.contains("RAFI_MIN_AGE"),
            "values stay in the log: {body}"
        );
    }

    #[test]
    fn release_builds_commit_only_to_public_chains() {
        let quicknet = chains::QUICKNET_HASH;
        let mock = rafi_raffle::drand::mock_chain().chain().hash();
        assert_eq!(
            resolve_chain(&quicknet.to_string(), None).unwrap(),
            quicknet
        );
        assert!(resolve_chain("mock", None).is_err());
        assert!(resolve_chain(&mock.to_string(), None).is_err());
        assert!(resolve_chain("not a hash", None).is_err());
        // Development builds also take the mock chain, by name or hash.
        assert_eq!(resolve_chain("mock", Some(mock)).unwrap(), mock);
        assert_eq!(resolve_chain(&mock.to_string(), Some(mock)).unwrap(), mock);
    }

    #[test]
    fn production_needs_an_operator_but_no_documents() {
        let cfg = Config::from_lookup(|_| None);
        assert!(cfg.validate().is_err(), "no operator");
        let dev = Config::from_lookup(|name| (name == "RAFI_DEV_AUTH").then(|| "1".into()));
        assert!(dev.validate().is_ok());
        assert_eq!(dev.documents, Documents::default(), "no placeholders");
        let mut full = Config::from_lookup(|name| match name {
            "RAFI_OPERATOR_NAME" => Some("Op".into()),
            "RAFI_OPERATOR_EMAIL" => Some("p@op.example".into()),
            "CLERK_PUBLISHABLE_KEY" => Some(TEST_CLERK_PUBLISHABLE_KEY.into()),
            _ => None,
        });
        assert!(
            full.validate().is_ok(),
            "no documents, and a random key is long enough"
        );
        assert_eq!(full.hosting, Hosting::Native);
        // Without Clerk's publishable key the admin has no sign-in to offer,
        // while the server expects one: refused, like a missing operator.
        let key = full.clerk_publishable_key.take();
        let err = full.validate().unwrap_err().to_string();
        assert!(err.contains("CLERK_PUBLISHABLE_KEY"), "{err}");
        // Nor one no Frontend API host decodes from (the secret key, say).
        for wrong in ["pk_test_rafi", "sk_test_abc", "pk_live_bm90IGEgaG9zdDsk"] {
            full.clerk_publishable_key = Some(wrong.into());
            let err = full.validate().unwrap_err().to_string();
            assert!(
                err.contains("not a Clerk publishable key"),
                "{wrong}: {err}"
            );
        }
        full.clerk_publishable_key = key;
        full.pow_key = "short".into();
        let err = full.validate().unwrap_err().to_string();
        assert!(
            err.contains("RAFI_POW_KEY"),
            "a short key is refused: {err}"
        );
        full.pow_key = "x".repeat(MIN_POW_KEY_CHARS);
        assert!(full.validate().is_ok());
        full.retention.days = 31;
        assert!(full.validate().is_err(), "default over the maximum");
        assert!(Config::for_tests().validate().is_ok());
    }

    #[test]
    fn documents_come_from_the_directory_each_optional() {
        let dir = std::env::temp_dir().join(format!("rafi-docs-{}", uuid::Uuid::new_v4()));
        // No directory at all (RAFI_DOCS_DIR pointing nowhere): a mistake.
        let err = format!("{:#}", Documents::from_dir(&dir).unwrap_err());
        assert!(
            err.contains(&dir.display().to_string()) && err.contains("not a directory"),
            "names the directory: {err}"
        );
        // An empty one: neither document.
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Documents::from_dir(&dir).unwrap(), Documents::default());
        // One of the two: that one alone.
        std::fs::write(
            dir.join("terms.md"),
            "---\nversion: 2026-11\n---\n# Terms\n",
        )
        .unwrap();
        let docs = Documents::from_dir(&dir).unwrap();
        let terms = docs.terms.expect("the terms");
        assert_eq!(terms.text, "# Terms\n");
        assert_eq!(terms.version, "2026-11");
        assert_eq!(docs.privacy, None);
        // Both.
        std::fs::write(
            dir.join("privacy.md"),
            "---\nversion: 2026-10\n---\n# Privacy\n",
        )
        .unwrap();
        let docs = Documents::from_dir(&dir).unwrap();
        assert_eq!(docs.terms.unwrap().version, "2026-11");
        let privacy = docs.privacy.expect("the privacy notice");
        assert_eq!(privacy.version, "2026-10");
        assert_eq!(privacy.text, "# Privacy\n");
        // Without its version, the server doesn't start, under dev auth too
        // (Config::from_env reads RAFI_DOCS_DIR with this): the error names
        // the file.
        std::fs::write(dir.join("privacy.md"), "# Privacy\n").unwrap();
        let err = format!("{:#}", Documents::from_dir(&dir).unwrap_err());
        let privacy = dir.join("privacy.md").display().to_string();
        assert!(
            err.starts_with(&format!("{privacy}: no front matter")),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_document_names_its_version_in_front_matter() {
        let parse = |source: &str| Document::parse("terms.md", source);
        // The front matter is read and isn't part of what is rendered.
        let d = parse("---\nversion: 2026-11\n---\n# Terms\n\nText.\n").unwrap();
        assert_eq!(d.version, "2026-11");
        assert_eq!(d.text, "# Terms\n\nText.\n");
        let page = crate::pages::legal("Terms", Some((&d.version, &d.text)), "terms");
        assert!(page.contains("Terms · version 2026-11"), "{page}");
        assert!(
            !page.contains("version:") && !page.contains("<hr"),
            "{page}"
        );
        // CRLF line endings, whitespace, blank lines, quotes and a byte order
        // mark are read the same.
        let d =
            parse("\u{feff}--- \r\n\r\n  version :  \"2026-11\" \r\n---\r\n# Terms\r\n").unwrap();
        assert_eq!(d.version, "2026-11");
        assert_eq!(d.text, "# Terms\r\n");
        assert_eq!(parse("---\nversion: '1'\n---").unwrap().text, "");
        // Refused, naming the file and what is wrong.
        for (source, why) in [
            ("# Terms\n", "no front matter"),
            ("", "no front matter"),
            ("\n---\nversion: 1\n---\n", "no front matter"),
            ("---\n---\n# Terms\n", "has no version"),
            ("---\nversion:\n---\n", "version is empty"),
            ("---\nversion:   \n---\n", "version is empty"),
            ("---\nversion: \"\"\n---\n", "version is empty"),
            ("---\nversion: 1\n", "no closing ---"),
            ("---\nversion: 1\nversion: 2\n---\n", "the version twice"),
            ("---\nverison: 1\n---\n", "\"verison\", which isn't a key"),
            (
                "---\nversion: 1\ntitle: Terms\n---\n",
                "\"title\", which isn't a key",
            ),
            (
                "---\nversion 1\n---\n",
                "line 2 of its front matter is not key: value",
            ),
        ] {
            let err = parse(source).unwrap_err().to_string();
            assert!(err.starts_with("terms.md: "), "{source:?}: {err}");
            assert!(err.contains(why), "{source:?}: {err}");
            assert!(err.contains(FRONT_MATTER), "{source:?}: {err}");
        }
    }

    #[test]
    fn retention_and_age_come_from_the_environment() {
        let cfg = Config::from_lookup(|name| match name {
            "RAFI_RETENTION_DAYS" => Some("3".into()),
            "RAFI_RETENTION_MAX_DAYS" => Some("10".into()),
            "RAFI_RETENTION_WINNERS_DAYS" => Some("60".into()),
            "RAFI_MIN_AGE" => Some("18".into()),
            _ => None,
        });
        assert_eq!(
            cfg.retention,
            RetentionPolicy {
                days: 3,
                max_days: 10,
                winner_hold_days: 60
            }
        );
        assert_eq!(cfg.min_age, 18);
        let defaults = Config::from_lookup(|_| None);
        assert_eq!(defaults.retention.days, DEFAULT_RETENTION_DAYS);
        assert_eq!(defaults.min_age, DEFAULT_MIN_AGE);
        let typo = Config::from_lookup(|name| match name {
            "RAFI_RETENTION_DAYS" => Some("7d".into()),
            "RAFI_DEV_AUTH" => Some("1".into()),
            _ => None,
        });
        let err = typo.validate().unwrap_err().to_string();
        assert!(err.contains("RAFI_RETENTION_DAYS=\"7d\""), "{err}");
        // Every numeric variable refuses a typo the same way, instead of
        // quietly taking its default.
        for (name, default) in [
            ("RAFI_ENGINE_TIMEOUT_SECS", "90"),
            ("RAFI_POW_COST", "1000"),
            ("RAFI_MAX_ENTRANTS", "2000"),
        ] {
            let typo = Config::from_lookup(|n| match n {
                n if n == name => Some("ninety".into()),
                "RAFI_DEV_AUTH" => Some("1".into()),
                _ => None,
            });
            let err = typo.validate().unwrap_err().to_string();
            assert!(err.contains(&format!("{name}=\"ninety\"")), "{name}: {err}");
            let set = Config::from_lookup(|n| (n == name).then(|| default.into()));
            assert!(set.misparsed.is_empty(), "{name}: {default} parses");
        }
    }

    #[test]
    fn the_policy_is_enforced_unless_reporting_is_asked_for() {
        let mode = |v: Option<&str>| {
            let cfg = Config::from_lookup(|name| match name {
                "RAFI_CSP" => v.map(Into::into),
                "RAFI_DEV_AUTH" => Some("1".into()),
                _ => None,
            });
            cfg.validate().map(|()| cfg.csp)
        };
        assert_eq!(mode(None).unwrap(), CspMode::Enforce);
        assert_eq!(mode(Some("enforce")).unwrap(), CspMode::Enforce);
        assert_eq!(mode(Some("report-only")).unwrap(), CspMode::ReportOnly);
        assert_eq!(mode(Some(" Report-Only ")).unwrap(), CspMode::ReportOnly);
        // A typo is refused, not taken for either.
        let err = mode(Some("off")).unwrap_err().to_string();
        assert!(err.contains("RAFI_CSP=\"off\""), "{err}");
    }

    #[test]
    fn the_chain_defaults_to_quicknet() {
        let cfg = Config::from_lookup(|_| None);
        assert_eq!(cfg.chain().unwrap(), chains::QUICKNET_HASH);
        let cfg = Config::from_lookup(|name| (name == "RAFI_DRAND_MOCK").then(|| "1".into()));
        assert_eq!(cfg.drand_chain, "mock");
    }
}
