//! The storage conformance suite: the contract every backend of [`Db`] must
//! pass.
//!
//! The store's SQL is written once, for SQLite (and D1) and Postgres alike,
//! and the app trusts the query layer to behave the same everywhere. These
//! scenarios pin down what "the same" means. They come in two groups, named
//! by prefix:
//!
//! - **`query.*`, the query layer:** decoding every type rafi reads
//!   (booleans stored as 0/1, `NULL`s, `NUMERIC` sums), `NULL` parameters
//!   where the column types them and where nothing does
//!   (`(?N IS NULL OR …)`), [`Db::batch`]'s atomicity and rollback,
//!   [`Db::batch_rows`] as one snapshot, unique violations as
//!   [`Error::Unique`](crate::Error::Unique), zero versus non-zero change
//!   counts (with triggers), ties in `ORDER BY`, a statement at D1's limit of
//!   100 bound parameters, and the version triggers;
//! - **`domain.*`, the domain functions:** the freeze's steps, joins (the
//!   cap, one entry per email, racing joins), fake entrants across
//!   statements, erasure and its log, retention, ALTCHA replays, the full
//!   view, the participant snapshot and [`save_drawing`](crate::save_drawing).
//!
//! Scenarios use only the public API ([`Db`], [`Query`](crate::Query) and the
//! domain modules), never a backend's internals, and each runs on a fresh,
//! empty, migrated database. A scenario that needs something not every
//! backend offers declares it as a [`Capabilities`] flag, and is skipped,
//! with the reason, where it's missing.
//!
//! # Where it runs
//!
//! - `cargo test -p rafi-store` (`tests/conformance.rs`) runs every scenario
//!   against in-memory SQLite ([`Sqlite`]) and, when `RAFI_TEST_POSTGRES_URL`
//!   names a throwaway database, Postgres ([`Postgres`], a fresh schema per
//!   scenario), one test each, named `<backend>::<scenario>`
//!   (`sqlite::domain.join_cap`).
//! - `crates/store-d1-check` runs them against D1 under `wrangler dev`
//!   (`scripts/workers-check.sh`), and answers with each scenario's outcome.
//!
//! # Adding a backend
//!
//! 1. Implement it in `backend/` and make [`Db`] open it.
//! 2. Write a [`Harness`]: a name, its [`Capabilities`], and
//!    [`Harness::fresh`], which returns an empty, migrated database each time
//!    (a new in-memory database, a new schema, or emptied tables).
//! 3. Run [`SCENARIOS`] through it: add it to `tests/conformance.rs` if it
//!    runs natively, or call [`run_all`] from wherever it runs (as
//!    `store-d1-check` does). It is done when every scenario passes or is
//!    skipped for a capability it really lacks.
//!
//! A new requirement on the query layer is a new `query.*` scenario here,
//! not a backend-specific test.

mod domain;
mod query;

use std::{fmt, future::Future, pin::Pin};

use crate::Db;

/// What a scenario returns: `Err` says what went wrong.
pub type Outcome = Result<(), String>;

/// What a backend can do beyond the baseline every backend offers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Capabilities(u32);

impl Capabilities {
    /// The baseline only.
    pub const NONE: Self = Self(0);
    /// Several connections at once, whose transactions really overlap (and
    /// can conflict), as with Postgres. In-memory SQLite has one connection;
    /// D1 runs one statement or batch at a time.
    pub const CONCURRENT_SESSIONS: Self = Self(1);

    const NAMES: &[(Self, &str)] = &[(Self::CONCURRENT_SESSIONS, "concurrent sessions")];

    /// Both sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every capability in `other` is in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The capabilities in `self` that `have` lacks, by name.
    fn missing_from(self, have: Self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(c, _)| self.contains(*c) && !have.contains(*c))
            .map(|(_, name)| *name)
            .collect()
    }
}

impl std::ops::BitOr for Capabilities {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

/// A backend under test: how to get a fresh database, and what it can do.
pub trait Harness {
    /// A short name, the prefix of each test's name (`sqlite`, `postgres`,
    /// `d1`).
    fn name(&self) -> &str;

    /// What the backend offers beyond the baseline.
    fn capabilities(&self) -> Capabilities;

    /// An empty, migrated database, unaffected by any earlier scenario. `Err`
    /// says why there is none.
    fn fresh(&self) -> impl Future<Output = Result<Db, String>>;
}

type Run = for<'a> fn(&'a Db) -> Pin<Box<dyn Future<Output = Outcome> + 'a>>;

/// A named scenario, and what it needs from the backend.
pub struct Scenario {
    /// `query.<what>` or `domain.<what>`.
    pub name: &'static str,
    /// What the backend must offer; elsewhere the scenario is skipped.
    pub needs: Capabilities,
    run: Run,
}

impl fmt::Debug for Scenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scenario")
            .field("name", &self.name)
            .field("needs", &self.needs)
            .finish_non_exhaustive()
    }
}

/// How a scenario went on a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    /// It ran, and this went wrong.
    Failed(String),
    /// It didn't run, for this reason (a capability the backend lacks).
    Skipped(String),
}

impl Scenario {
    /// Why the scenario can't run on a backend with `have`, if it can't
    /// ("needs concurrent sessions").
    pub fn skip_reason(&self, have: Capabilities) -> Option<String> {
        let missing = self.needs.missing_from(have);
        (!missing.is_empty()).then(|| format!("needs {}", missing.join(", ")))
    }

    /// Runs the scenario on `db`, which must be fresh.
    pub fn run<'a>(&self, db: &'a Db) -> impl Future<Output = Outcome> + 'a {
        (self.run)(db)
    }

    /// Runs the scenario on a fresh database from `harness`, or skips it if
    /// the backend lacks something it needs.
    pub async fn run_on<H: Harness + ?Sized>(&self, harness: &H) -> Verdict {
        if let Some(reason) = self.skip_reason(harness.capabilities()) {
            return Verdict::Skipped(reason);
        }
        let db = match harness.fresh().await {
            Ok(db) => db,
            Err(e) => return Verdict::Failed(format!("a fresh database: {e}")),
        };
        match self.run(&db).await {
            Ok(()) => Verdict::Passed,
            Err(e) => Verdict::Failed(e),
        }
    }
}

macro_rules! scenarios {
    ($($name:literal $(needs $needs:expr)? => $f:path,)*) => {
        &[$(Scenario {
            name: $name,
            needs: Capabilities::NONE $(.union($needs))?,
            run: |db| Box::pin($f(db)),
        },)*]
    };
}

/// Every scenario, in the order they run.
pub static SCENARIOS: &[Scenario] = scenarios! {
    "query.decode_types" => query::decode_types,
    "query.null_params" => query::null_params,
    "query.batch_atomicity" => query::batch_atomicity,
    "query.batch_rows_snapshot" => query::batch_rows_snapshot,
    "query.batch_rows_under_writes" needs Capabilities::CONCURRENT_SESSIONS
        => query::batch_rows_under_writes,
    "query.unique_violation" => query::unique_violation,
    "query.change_counts" => query::change_counts,
    "query.ordering_ties" => query::ordering_ties,
    "query.param_limit" => query::param_limit,
    "query.version_triggers" => query::version_triggers,
    "domain.save_drawing" => domain::keeps_the_newest_state,
    "domain.freeze_steps" => domain::freezes_in_steps,
    "domain.join_cap" => domain::joins_up_to_the_cap_once_per_email,
    "domain.join_cap_race" needs Capabilities::CONCURRENT_SESSIONS
        => domain::racing_joins_respect_the_cap,
    "domain.fakes_across_statements" => domain::adds_fakes_in_chunks,
    "domain.erasure_log" => domain::erases_and_logs_erasures,
    "domain.retention" => domain::retention_erases_what_is_due,
    "domain.altcha_replay" => domain::altcha_refuses_replays,
    "domain.full_view" => domain::reads_the_full_view,
    "domain.participant_snapshot" => domain::phones_see_only_drawn_names,
    "domain.frozen_list" => domain::the_list_and_prizes_are_fixed_once_frozen,
    "domain.entrant_patch" => domain::patches_an_entrant_at_once,
};

/// Runs every scenario through `harness`, one after the other.
pub async fn run_all<H: Harness + ?Sized>(harness: &H) -> Vec<(&'static str, Verdict)> {
    let mut verdicts = Vec::with_capacity(SCENARIOS.len());
    for scenario in SCENARIOS {
        verdicts.push((scenario.name, scenario.run_on(harness).await));
    }
    verdicts
}

// ---------------------------------------------------------------------------
// Helpers for the scenarios
// ---------------------------------------------------------------------------

fn check(ok: bool, what: &str) -> Outcome {
    if ok { Ok(()) } else { Err(what.to_owned()) }
}

fn fail(e: impl fmt::Display) -> String {
    e.to_string()
}

// ---------------------------------------------------------------------------
// The native harnesses
// ---------------------------------------------------------------------------

/// In-memory SQLite: a new database per scenario. Always available.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, Default)]
pub struct Sqlite;

#[cfg(not(target_arch = "wasm32"))]
impl Harness for Sqlite {
    fn name(&self) -> &str {
        "sqlite"
    }

    /// One connection: nothing overlaps.
    fn capabilities(&self) -> Capabilities {
        Capabilities::NONE
    }

    async fn fresh(&self) -> Result<Db, String> {
        Db::connect("sqlite::memory:").await.map_err(fail)
    }
}

/// Postgres: a new schema per scenario in the throwaway database
/// `RAFI_TEST_POSTGRES_URL` names. The schemas are left behind.
#[cfg(all(feature = "postgres", not(target_arch = "wasm32")))]
#[derive(Debug, Clone)]
pub struct Postgres {
    base_url: String,
}

#[cfg(all(feature = "postgres", not(target_arch = "wasm32")))]
impl Postgres {
    /// The database `RAFI_TEST_POSTGRES_URL` names; `Err` says it isn't set.
    pub fn from_env() -> Result<Self, String> {
        crate::testing::postgres_base_url()
            .map(|base_url| Self { base_url })
            .ok_or_else(|| format!("{} not set", crate::testing::POSTGRES_URL_VAR))
    }
}

#[cfg(all(feature = "postgres", not(target_arch = "wasm32")))]
impl Harness for Postgres {
    fn name(&self) -> &str {
        "postgres"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::CONCURRENT_SESSIONS
    }

    async fn fresh(&self) -> Result<Db, String> {
        fresh_schema(&self.base_url).await
    }
}

/// A new, migrated schema in the Postgres database at `base_url`.
#[cfg(all(feature = "postgres", not(target_arch = "wasm32")))]
async fn fresh_schema(base_url: &str) -> Result<Db, String> {
    let url = crate::testing::fresh_schema(base_url)
        .await
        .map_err(|e| format!("creating a schema: {e}"))?;
    let db = Db::connect(&url).await.map_err(fail)?;
    check(
        db.dialect() == crate::Dialect::Postgres,
        "the URL opened Postgres",
    )?;
    Ok(db)
}

/// An embedded PostgreSQL (Oliphaunt: a local server from prebuilt native
/// binaries, no install needed). One server for the whole run, started on
/// first use, with a new schema per scenario, as for [`Postgres`].
/// `RAFI_TEST_EMBEDDED_POSTGRES=off` turns it off.
///
/// The server is a child process: [`EmbeddedPostgres::stop`] it when done,
/// since a runner that ends with `process::exit` drops nothing.
#[cfg(all(feature = "embedded-postgres", not(target_arch = "wasm32")))]
#[derive(Default)]
pub struct EmbeddedPostgres {
    url: std::sync::OnceLock<Result<String, String>>,
    server: std::sync::Mutex<Option<oliphaunt::OliphauntServer>>,
}

#[cfg(all(feature = "embedded-postgres", not(target_arch = "wasm32")))]
impl EmbeddedPostgres {
    /// The variable that turns the embedded Postgres off (`off`).
    pub const VAR: &'static str = "RAFI_TEST_EMBEDDED_POSTGRES";

    /// `Err` says it's turned off. Starts nothing yet.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var(Self::VAR).as_deref() {
            Ok("off" | "0" | "false") => Err(format!("turned off by {}", Self::VAR)),
            _ => Ok(Self::default()),
        }
    }

    /// The server's URL, starting it on the first call (blocking, a few
    /// seconds). A failed start fails every scenario with the same reason.
    fn url(&self) -> Result<&str, String> {
        self.url
            .get_or_init(|| {
                let server = oliphaunt::OliphauntServer::builder()
                    .listen(oliphaunt::ServerListen::tcp())
                    .start()
                    .map_err(|e| format!("starting the embedded Postgres: {e}"))?;
                let url = server.connection_string().to_owned();
                *self.lock() = Some(server);
                Ok(url)
            })
            .as_deref()
            .map_err(Clone::clone)
    }

    /// Stops the server, if it was started.
    pub fn stop(&self) -> Result<(), String> {
        let server = self.lock().take();
        match server {
            Some(mut server) => server
                .close()
                .map_err(|e| format!("stopping the embedded Postgres: {e}")),
            None => Ok(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<oliphaunt::OliphauntServer>> {
        self.server
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(all(feature = "embedded-postgres", not(target_arch = "wasm32")))]
impl Harness for EmbeddedPostgres {
    fn name(&self) -> &str {
        "embedded-postgres"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::CONCURRENT_SESSIONS
    }

    async fn fresh(&self) -> Result<Db, String> {
        fresh_schema(self.url()?).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_names_are_unique_and_grouped() {
        let mut names: Vec<_> = SCENARIOS.iter().map(|s| s.name).collect();
        assert!(
            names
                .iter()
                .all(|n| n.starts_with("query.") || n.starts_with("domain.")),
            "{names:?}"
        );
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SCENARIOS.len(), "duplicate scenario names");
    }

    #[test]
    fn capabilities_name_what_is_missing() {
        let race = SCENARIOS
            .iter()
            .find(|s| s.name == "domain.join_cap_race")
            .unwrap();
        assert_eq!(
            race.skip_reason(Capabilities::NONE).as_deref(),
            Some("needs concurrent sessions")
        );
        assert_eq!(race.skip_reason(Capabilities::CONCURRENT_SESSIONS), None);
        assert!(Capabilities::NONE.contains(Capabilities::NONE));
        assert!(!Capabilities::NONE.contains(Capabilities::CONCURRENT_SESSIONS));
    }
}
