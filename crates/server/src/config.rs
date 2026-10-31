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

/// The installation's legal documents: its own privacy notice and its terms
/// (with the data processing agreement, which organizers accept by using the
/// service), as Markdown, each with the version the deployment gives it
/// (`RAFI_PRIVACY_VERSION`, `RAFI_TERMS_VERSION`). Natively they are `privacy.md` and `terms.md` in
/// `RAFI_DOCS_DIR`; on Workers, `legal/privacy.md` and `legal/terms.md` among the Worker's static files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Documents {
    pub terms: String,
    pub privacy: String,
    /// The built-in placeholders: development only.
    pub placeholder: bool,
}

impl Documents {
    /// The line the repository's templates (`deploy/cloudflare/legal/`) carry
    /// until an operator has reviewed them. A document that still has it is a
    /// placeholder, which a production server refuses.
    pub const TEMPLATE_MARKER: &str = "**Template.**";

    /// The documents from `terms` and `privacy`, as a deployment supplies
    /// them. An unedited template counts as a placeholder.
    pub fn new(terms: impl Into<String>, privacy: impl Into<String>) -> Self {
        let (terms, privacy) = (terms.into(), privacy.into());
        let placeholder =
            terms.contains(Self::TEMPLATE_MARKER) || privacy.contains(Self::TEMPLATE_MARKER);
        Self {
            terms,
            privacy,
            placeholder,
        }
    }

    /// Built-in placeholders, marked as such: for development under dev auth
    /// only. A production server refuses to start with them.
    pub fn placeholders() -> Self {
        Self {
            terms: PLACEHOLDER_TERMS.into(),
            privacy: PLACEHOLDER_PRIVACY.into(),
            placeholder: true,
        }
    }

    /// `terms.md` and `privacy.md` from `dir`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_dir(dir: &std::path::Path) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        let read = |name: &str| {
            let path = dir.join(name);
            std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
        };
        Ok(Self::new(read("terms.md")?, read("privacy.md")?))
    }
}

const PLACEHOLDER_TERMS: &str = "# Terms (placeholder)\n\n**This is a development placeholder, not a legal document.** \
A deployment supplies its own terms, with the data processing agreement, as `terms.md` in `RAFI_DOCS_DIR` \
(natively) or `legal/terms.md` among the Worker's static files.\n";

const PLACEHOLDER_PRIVACY: &str = "# Privacy notice (placeholder)\n\n**This is a development placeholder, not a legal document.** \
A deployment supplies the installation's own privacy notice (the operator, its subprocessors: hosting, \
organizer sign-in, the raffle engine, the randomness network) as `privacy.md` in `RAFI_DOCS_DIR` (natively) or \
`legal/privacy.md` among the Worker's static files.\n";

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
    /// The installation's legal documents.
    pub documents: Documents,
    /// Their versions (`RAFI_TERMS_VERSION`, `RAFI_PRIVACY_VERSION`): what
    /// organizers accept, and what the notices show.
    pub terms_version: String,
    pub privacy_version: String,
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
                ingress_url: var("RAFI_ENGINE_INGRESS_URL")
                    .unwrap_or_default()
                    .trim_end_matches('/')
                    .to_owned(),
                api_key: var("RAFI_ENGINE_API_KEY").unwrap_or_default(),
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
            documents: Documents::placeholders(),
            terms_version: var("RAFI_TERMS_VERSION").unwrap_or_else(|| dev_version(dev_auth)),
            privacy_version: var("RAFI_PRIVACY_VERSION").unwrap_or_else(|| dev_version(dev_auth)),
            docs_dir: var("RAFI_DOCS_DIR"),
            misparsed: misparsed.into_inner(),
        }
    }

    /// Whether the settings make sense together, and a production server
    /// (no dev auth) has everything it must: an operator, document versions
    /// and real documents.
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
        if self.terms_version.is_empty() || self.privacy_version.is_empty() {
            bail!("RAFI_TERMS_VERSION and RAFI_PRIVACY_VERSION are required");
        }
        if self.documents.placeholder {
            bail!(
                "the legal documents are placeholders or unreviewed templates (they still say {:?}): supply reviewed terms.md and privacy.md (RAFI_DOCS_DIR natively; legal/ of the installation's deploy/cloudflare on Workers)",
                Documents::TEMPLATE_MARKER
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
            documents: Documents::new(
                "# Test terms\n\nThe test terms, with the **processing agreement**.\n",
                "# Test privacy notice\n\nThe test installation's notice.\n",
            ),
            terms_version: "test-1".into(),
            privacy_version: "test-1".into(),
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

/// The document version under dev auth when none is set.
fn dev_version(dev_auth: bool) -> String {
    if dev_auth {
        "dev".into()
    } else {
        String::new()
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
    fn production_needs_an_operator_and_documents() {
        let cfg = Config::from_lookup(|_| None);
        assert!(cfg.validate().is_err(), "no operator, no documents");
        let dev = Config::from_lookup(|name| (name == "RAFI_DEV_AUTH").then(|| "1".into()));
        assert!(dev.validate().is_ok(), "placeholders under dev auth");
        assert!(dev.documents.placeholder);
        assert_eq!(dev.terms_version, "dev");
        let mut full = Config::from_lookup(|name| match name {
            "RAFI_OPERATOR_NAME" => Some("Op".into()),
            "RAFI_OPERATOR_EMAIL" => Some("p@op.example".into()),
            "RAFI_TERMS_VERSION" | "RAFI_PRIVACY_VERSION" => Some("2026-10".into()),
            "CLERK_PUBLISHABLE_KEY" => Some(TEST_CLERK_PUBLISHABLE_KEY.into()),
            _ => None,
        });
        assert!(full.validate().is_err(), "placeholder documents");
        full.documents = Documents::new("# T\n\n> **Template.** Review me.", "# P");
        assert!(full.validate().is_err(), "an unedited template");
        full.documents = Documents::new("# T", "# P");
        assert!(full.validate().is_ok(), "a random key is long enough");
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
    fn documents_come_from_the_directory_or_the_server_doesnt_start() {
        let dir = std::env::temp_dir().join(format!("rafi-docs-{}", uuid::Uuid::new_v4()));
        // No directory at all (RAFI_DOCS_DIR pointing nowhere).
        let err = Documents::from_dir(&dir).unwrap_err();
        assert!(
            format!("{err:#}").contains(&dir.join("terms.md").display().to_string()),
            "names the file it couldn't read: {err:#}"
        );
        // One of the two missing: refused, naming the missing one, never
        // half a set with a placeholder for the other.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("terms.md"), "# Terms\n").unwrap();
        let err = format!("{:#}", Documents::from_dir(&dir).unwrap_err());
        assert!(
            err.contains(&dir.join("privacy.md").display().to_string()),
            "{err}"
        );
        // Both present: the documents, an unedited template still a placeholder.
        std::fs::write(
            dir.join("privacy.md"),
            "# Privacy\n\n> **Template.** Review me.\n",
        )
        .unwrap();
        let docs = Documents::from_dir(&dir).unwrap();
        assert_eq!(docs.terms, "# Terms\n");
        assert!(
            docs.placeholder,
            "the template marker is seen through a file too"
        );
        std::fs::write(dir.join("privacy.md"), "# Privacy\n").unwrap();
        let docs = Documents::from_dir(&dir).unwrap();
        assert!(!docs.placeholder);
        assert_eq!(docs.privacy, "# Privacy\n");
        std::fs::remove_dir_all(&dir).unwrap();
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
