//! The hook in a VirtualDom on the host, solving with the `native` feature.
#![cfg(not(target_arch = "wasm32"))]

use std::{cell::RefCell, convert::Infallible, time::Duration};

use dioxus::{
    core::{NoOpMutations, VirtualDom},
    prelude::*,
};
use dioxus_altcha::{
    AltchaField, AltchaOptions, AltchaState, Auto, RawChallenge, UseAltcha, decode_payload,
    use_altcha, use_altcha_with_options,
};

const KEY: &str = "hook-test-key";

thread_local! {
    static HANDLE: RefCell<Option<UseAltcha>> = const { RefCell::new(None) };
    static FETCHES: RefCell<u32> = const { RefCell::new(0) };
    static SOLVED: RefCell<Option<Result<String, dioxus_altcha::AltchaError>>> = const { RefCell::new(None) };
}

async fn issue() -> Result<RawChallenge, Infallible> {
    FETCHES.with(|f| *f.borrow_mut() += 1);
    let challenge = altcha::create_challenge(altcha::CreateChallengeOptions {
        cost: 1,
        key_prefix: "0".into(),
        hmac_signature_secret: Some(KEY.into()),
        expires_at: Some(4_000_000_000),
        ..Default::default()
    })
    .unwrap();
    Ok(RawChallenge::from_challenge(&challenge))
}

fn verifies(payload: &str) -> bool {
    let p = decode_payload(payload).unwrap();
    altcha::verify_solution(altcha::VerifySolutionOptions::new(
        &p.challenge,
        &p.solution,
        KEY,
    ))
    .unwrap()
    .verified
}

fn handle() -> UseAltcha {
    HANDLE.with(|h| h.borrow().unwrap())
}

fn solved() -> bool {
    matches!(*handle().signal().peek(), AltchaState::Solved { .. })
}

/// Runs the dom until `done` (checked in its runtime) or 10 s.
async fn run_until(dom: &mut VirtualDom, mut done: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !dom.in_runtime(&mut done) {
            tokio::select! {
                () = dom.wait_for_work() => {}
                () = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
            dom.render_immediate(&mut NoOpMutations);
        }
    })
    .await
    .expect("timed out");
}

fn on_mount() -> Element {
    let altcha = use_altcha(issue);
    HANDLE.with(|h| *h.borrow_mut() = Some(altcha));
    rsx! {
        form { AltchaField { altcha } }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn solves_on_mount() {
    let mut dom = VirtualDom::new(on_mount);
    dom.rebuild_in_place();
    // The first render is Idle with an empty field: what SSR sends.
    assert_eq!(dom.in_runtime(|| handle().state()), AltchaState::Idle);
    assert_eq!(
        dioxus_ssr::render(&dom),
        r#"<form><input type="hidden" name="altcha" value=""/></form>"#
    );

    run_until(&mut dom, solved).await;
    let payload = dom.in_runtime(|| handle().payload()).unwrap();
    assert!(verifies(&payload));
    assert!(dioxus_ssr::render(&dom).contains(&format!(r#"value="{payload}""#)));

    // A refresh spends nothing twice: a new challenge, a new payload.
    let fetches = FETCHES.with(|f| *f.borrow());
    dom.in_runtime(|| handle().refresh());
    run_until(&mut dom, || {
        FETCHES.with(|f| *f.borrow()) > fetches && solved()
    })
    .await;
    let again = dom.in_runtime(|| handle().payload()).unwrap();
    assert_ne!(again, payload);
    assert!(verifies(&again));
}

fn on_submit() -> Element {
    let altcha = use_altcha_with_options(issue, AltchaOptions::new().auto(Auto::Off));
    HANDLE.with(|h| *h.borrow_mut() = Some(altcha));
    use_hook(|| {
        spawn(async move {
            let result = altcha.solved().await;
            SOLVED.with(|s| *s.borrow_mut() = Some(result));
        })
    });
    rsx! {}
}

#[tokio::test(flavor = "current_thread")]
async fn solves_on_demand() {
    FETCHES.with(|f| *f.borrow_mut() = 0);
    let mut dom = VirtualDom::new(on_submit);
    dom.rebuild_in_place();
    run_until(&mut dom, || SOLVED.with(|s| s.borrow().is_some())).await;
    let payload = SOLVED.with(|s| s.borrow_mut().take()).unwrap().unwrap();
    assert!(verifies(&payload));
    assert_eq!(FETCHES.with(|f| *f.borrow()), 1);
    // Solved and usable: start() doesn't fetch again.
    dom.in_runtime(|| handle().start());
    dom.render_immediate(&mut NoOpMutations);
    assert_eq!(FETCHES.with(|f| *f.borrow()), 1);
    // With Auto::Off, a refresh goes Idle.
    dom.in_runtime(|| handle().refresh());
    assert_eq!(dom.in_runtime(|| handle().state()), AltchaState::Idle);
}

async fn issue_short() -> Result<RawChallenge, Infallible> {
    FETCHES.with(|f| *f.borrow_mut() += 1);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let challenge = altcha::create_challenge(altcha::CreateChallengeOptions {
        cost: 1,
        key_prefix: "0".into(),
        hmac_signature_secret: Some(KEY.into()),
        expires_at: Some(now + 2),
        ..Default::default()
    })
    .unwrap();
    Ok(RawChallenge::from_challenge(&challenge))
}

fn short_lived() -> Element {
    let altcha = use_altcha(issue_short);
    HANDLE.with(|h| *h.borrow_mut() = Some(altcha));
    rsx! {}
}

#[tokio::test(flavor = "current_thread")]
async fn solves_again_when_expired() {
    FETCHES.with(|f| *f.borrow_mut() = 0);
    let mut dom = VirtualDom::new(short_lived);
    dom.rebuild_in_place();
    run_until(&mut dom, solved).await;
    let first = dom.in_runtime(|| handle().payload()).unwrap();
    // Expires early (half the ~2 s left), then fetches and solves again.
    run_until(&mut dom, || FETCHES.with(|f| *f.borrow()) >= 2 && solved()).await;
    let second = dom.in_runtime(|| handle().payload()).unwrap();
    assert_ne!(first, second);
    assert!(verifies(&second));
}

thread_local! {
    static REFRESHED: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

fn refresh_then_solved() -> Element {
    let altcha = use_altcha(issue);
    HANDLE.with(|h| *h.borrow_mut() = Some(altcha));
    use_hook(|| {
        spawn(async move {
            let spent = altcha.solved().await.unwrap();
            // As after a submit: the payload is spent, the next is fresh.
            altcha.refresh();
            let fresh = altcha.solved().await.unwrap();
            REFRESHED.with(|r| *r.borrow_mut() = Some((spent, fresh)));
        })
    });
    rsx! {}
}

#[tokio::test(flavor = "current_thread")]
async fn solved_after_refresh_is_fresh() {
    let mut dom = VirtualDom::new(refresh_then_solved);
    dom.rebuild_in_place();
    run_until(&mut dom, || REFRESHED.with(|r| r.borrow().is_some())).await;
    let (spent, fresh) = REFRESHED.with(|r| r.borrow_mut().take()).unwrap();
    assert_ne!(
        spent, fresh,
        "refresh() then solved() returned the spent payload"
    );
    assert!(verifies(&fresh));
}
