//! `solve` in a real browser: workers, the main-thread fallback, the timeout
//! and cancellation.
//!
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!   GECKODRIVER=geckodriver cargo test -p dioxus-altcha --target wasm32-unknown-unknown --test wasm_solve
//!
//! (or CHROMEDRIVER / SAFARIDRIVER). Tests share the page, so each restores
//! what it patches.
#![cfg(target_arch = "wasm32")]

use std::time::Duration;

use altcha::{CreateChallengeOptions, VerifySolutionOptions};
use dioxus_altcha::{RawChallenge, SolveError, SolveOptions, solve};
use futures_util::future::{Either, select};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const KEY: &str = "wasm-test-key";

fn issue(cost: u32, key_prefix: &str) -> altcha::Challenge {
    altcha::create_challenge(CreateChallengeOptions {
        cost,
        key_prefix: key_prefix.into(),
        hmac_signature_secret: Some(KEY.into()),
        ..Default::default()
    })
    .unwrap()
}

fn verified(challenge: &altcha::Challenge, solution: &altcha::Solution) -> bool {
    altcha::verify_solution(VerifySolutionOptions::new(challenge, solution, KEY))
        .unwrap()
        .verified
}

/// Counts `Worker.prototype.terminate` calls in `__terminated`.
fn count_terminations() {
    js_sys::eval(
        "if (!globalThis.__realTerminate) {
           globalThis.__realTerminate = Worker.prototype.terminate;
           Worker.prototype.terminate = function () {
             globalThis.__terminated++;
             return __realTerminate.call(this);
           };
         }
         globalThis.__terminated = 0;",
    )
    .unwrap();
}

fn terminated() -> f64 {
    js_sys::eval("globalThis.__terminated")
        .unwrap()
        .as_f64()
        .unwrap()
}

fn restore_worker() {
    js_sys::eval(
        "if (globalThis.__realTerminate) {
           Worker.prototype.terminate = __realTerminate;
           delete globalThis.__realTerminate;
         }
         if (globalThis.__RealWorker) { globalThis.Worker = __RealWorker; delete globalThis.__RealWorker; }",
    )
    .unwrap();
}

#[wasm_bindgen_test]
async fn solves_in_workers() {
    count_terminations();
    let challenge = issue(100, "00");
    let raw = RawChallenge::from_challenge(&challenge);
    let solution = solve(&raw, &SolveOptions::new().workers(3)).await.unwrap();
    assert!(verified(&challenge, &solution));
    // Every worker is stopped once one finds it.
    assert_eq!(terminated(), 3.0);
    restore_worker();
}

#[wasm_bindgen_test]
async fn dropping_terminates_the_workers() {
    count_terminations();
    // Far too hard to finish in 300 ms.
    let raw = RawChallenge::from_challenge(&issue(1000, "000000"));
    let opts = SolveOptions::new().workers(2);
    let work = Box::pin(solve(&raw, &opts));
    let tick = gloo_timers::future::TimeoutFuture::new(300);
    match select(work, tick).await {
        Either::Left((result, _)) => panic!("solved too soon: {result:?}"),
        Either::Right(((), work)) => drop(work),
    }
    assert_eq!(terminated(), 2.0);
    restore_worker();
}

#[wasm_bindgen_test]
async fn times_out() {
    count_terminations();
    let raw = RawChallenge::from_challenge(&issue(1000, "000000"));
    let opts = SolveOptions::new()
        .workers(2)
        .timeout(Some(Duration::from_millis(300)));
    assert_eq!(solve(&raw, &opts).await.unwrap_err(), SolveError::Timeout);
    assert_eq!(terminated(), 2.0);
    restore_worker();
}

#[wasm_bindgen_test]
async fn falls_back_to_the_main_thread() {
    // As if a CSP forbade workers.
    js_sys::eval(
        "globalThis.__RealWorker = Worker;
         globalThis.Worker = function () { throw new Error('blocked'); };",
    )
    .unwrap();
    let challenge = issue(100, "0");
    let raw = RawChallenge::from_challenge(&challenge);
    let result = solve(&raw, &SolveOptions::new()).await;
    restore_worker();
    assert!(verified(&challenge, &result.unwrap()));
}

#[wasm_bindgen_test]
async fn a_missing_worker_script_fails() {
    // Workers report `error`, the fallback can't load the script either.
    // (An earlier test may have loaded the solver into the page.)
    js_sys::eval("globalThis.altchaSolve = undefined;").unwrap();
    let raw = RawChallenge::from_challenge(&issue(1, "0"));
    let opts = SolveOptions::new().worker_url("/no-such-altcha-worker.js");
    let result = solve(&raw, &opts).await;
    assert!(matches!(result, Err(SolveError::Worker(_))), "{result:?}");
}

#[wasm_bindgen_test]
async fn refuses_unsupported_algorithms() {
    let raw = RawChallenge::from_json(r#"{"parameters":{"algorithm":"ARGON2ID"}}"#).unwrap();
    assert_eq!(
        solve(&raw, &SolveOptions::new()).await.unwrap_err(),
        SolveError::UnsupportedAlgorithm("ARGON2ID".into())
    );
}
