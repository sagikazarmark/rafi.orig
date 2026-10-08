//! The storage conformance suite (`rafi_store::conformance`) against every
//! native backend: one test per backend and scenario, named
//! `<backend>::<scenario>` (`sqlite::domain.join_cap`), each on a fresh
//! database.
//!
//! A backend that isn't available here (Postgres without
//! `RAFI_TEST_POSTGRES_URL`), or lacks a capability a scenario needs, reports
//! the test as ignored, with the reason. D1 runs the same scenarios through
//! `crates/store-d1-check`.

use std::sync::Arc;

use libtest_mimic::{Arguments, Completion, Failed, Trial};
use rafi_store::conformance::{self, Harness, SCENARIOS, Verdict};

/// One trial per scenario for `harness`, or, if it isn't available, one
/// ignored trial per scenario saying why.
fn trials<H>(name: &str, harness: Result<H, String>) -> Vec<Trial>
where
    H: Harness + Send + Sync + 'static,
{
    let harness = harness.map(Arc::new);
    SCENARIOS
        .iter()
        .map(|scenario| {
            let test = format!("{name}::{}", scenario.name);
            let harness = harness.clone();
            Trial::ignorable_test(test, move || {
                let harness = match harness {
                    Ok(h) => h,
                    Err(reason) => return Ok(Completion::ignored_with(reason)),
                };
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| Failed::from(format!("a runtime: {e}")))?;
                match runtime.block_on(scenario.run_on(harness.as_ref())) {
                    Verdict::Passed => Ok(Completion::Completed),
                    Verdict::Skipped(reason) => Ok(Completion::ignored_with(reason)),
                    Verdict::Failed(e) => Err(e.into()),
                }
            })
        })
        .collect()
}

fn main() {
    let args = Arguments::from_args();
    let mut tests = trials("sqlite", Ok(conformance::Sqlite));
    #[cfg(feature = "postgres")]
    tests.extend(trials("postgres", conformance::Postgres::from_env()));
    #[cfg(not(feature = "postgres"))]
    tests.extend(trials::<conformance::Sqlite>(
        "postgres",
        Err("rafi-store built without the `postgres` feature".into()),
    ));
    libtest_mimic::run(&args, tests).exit();
}
