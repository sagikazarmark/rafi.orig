//! Integration tests: in-memory SQLite (or, with `RAFI_TEST_POSTGRES_URL`
//! set, a fresh Postgres schema per test), the in-process raffle engine
//! (`rafi_engine::local`), mock drand, and a test's clock: it says the time
//! the test started until the test moves it (`app.clock`), and the engine's
//! writes follow it too. Drand's rounds are real time all the same.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use rafi_app::Limiter;
use rafi_lifecycle::RaffleState;
use rafi_live_client::Follow;
use rafi_live_server::{LiveUpdates, Recording};
use rafi_raffle::{ItemStatus, RaffleId, Resolution};
use rafi_store::{RaffleRow, entrants, prizes, raffles, view::Full};
use tower::ServiceExt;

use rafi_raffle::{Decision, DrawRequest};

use super::{AppState, config::Config, routes, service};
use rafi_api::{LifecycleCommand, PrizeInput};

const OWNER: &str = "user_test";
/// The test owner acting for themselves (a personal account).
const ACTOR: service::Actor<'static> = service::Actor::solo(OWNER);

async fn app() -> AppState {
    app_with(Config::for_tests()).await
}

async fn app_with(cfg: Config) -> AppState {
    app_with_live(cfg, LiveUpdates::polling()).await
}

/// The raffle engine's drand: the mock chain, no lead.
fn mock_drand() -> rafi_engine::drand::DrandConfig {
    rafi_engine::drand::DrandConfig {
        relays: vec![],
        mock: true,
        lead_secs: 0,
        timeout_secs: 5,
    }
}

async fn app_with_live(cfg: Config, live: LiveUpdates) -> AppState {
    app_with_engine(cfg, mock_drand(), live).await
}

/// An app whose raffle engine is the in-process one (drawing with `drand`),
/// writing into the same database by the same (test's) clock. With
/// `RAFI_TEST_POSTGRES_URL` set, the test database is a fresh schema there
/// instead of in-memory SQLite.
async fn app_with_engine(
    mut cfg: Config,
    drand: rafi_engine::drand::DrandConfig,
    live: LiveUpdates,
) -> AppState {
    if cfg.database_url == Config::for_tests().database_url
        && let Some(url) = rafi_store::testing::postgres_url().await
    {
        cfg.database_url = url;
    }
    let db = rafi_store::Db::connect(&cfg.database_url)
        .await
        .expect("database");
    let clock = service::Clock::fixed(service::Clock::system().now());
    let engine = rafi_engine::local::LocalEngine::new(
        rafi_engine::drand::DrandClient::builder(drand)
            .http_transport(drand::client::ReqwestTransport::default())
            .build()
            .expect("drand configuration"),
        rafi_store::RaffleWriter::with_clock(db.clone(), clock.clone()),
        rafi_live_server::Polling,
    );
    AppState::from_parts(
        cfg,
        db,
        Some(Arc::new(engine)),
        live,
        Arc::new(Limiter::default()),
        clock,
    )
    .expect("app state")
}

/// Live updates that record every call and tell clients to subscribe over
/// partyline.
fn recording() -> (Arc<Recording>, LiveUpdates) {
    let recording = Arc::new(Recording::default());
    let live = LiveUpdates::new(recording.clone(), |_, _| Follow::Partyline {
        base_url: None,
    });
    (recording, live)
}

async fn send(router: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    let resp = router.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

async fn solved_pow(router: &Router) -> String {
    let (_, _, body) = send(
        router,
        Request::get("/api/challenge").body(Body::empty()).unwrap(),
    )
    .await;
    solve_pow(serde_json::from_str(&body).expect("challenge json"))
}

/// The join form's value for `challenge`, solved from counter `from`.
fn solve_pow_from(challenge: altcha::Challenge, from: u32) -> altcha::Payload {
    let solution = altcha::solve_challenge(altcha::SolveChallengeOptions {
        counter_start: from,
        ..altcha::SolveChallengeOptions::new(&challenge)
    })
    .unwrap()
    .expect("solvable");
    altcha::Payload {
        challenge,
        solution,
    }
}

fn solve_pow(challenge: altcha::Challenge) -> String {
    service::pow::encode(&solve_pow_from(challenge, 0))
}

fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}={}",
                v.replace('%', "%25")
                    .replace('+', "%2B")
                    .replace('@', "%40")
                    .replace('=', "%3D")
                    .replace('/', "%2F")
                    .replace(' ', "+")
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Joins through the participant page; returns the entrant cookie (if set) and the response.
async fn join(
    router: &Router,
    code: &str,
    name: &str,
    email: &str,
) -> (StatusCode, Option<String>, String) {
    let proof = solved_pow(router).await;
    let body = form(&[
        ("name", name),
        ("email", email),
        ("rules", "on"),
        ("proof", &proof),
    ]);
    let (status, headers, text) = send(
        router,
        Request::post(format!("/j/{code}"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    let cookie = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("rafi_e_"))
        .map(|c| c.split(';').next().unwrap().to_owned());
    (status, cookie, text)
}

/// A raffle of `owner`'s titled `title`, dated today, with an organizer
/// named, so registration can open.
async fn create(app: &AppState, owner: &str, title: &str) -> String {
    let actor = service::Actor::solo(owner);
    let id = service::create(app, actor, title, &service::EventDate::today(app))
        .await
        .unwrap();
    name_organizer(app, actor, &id).await;
    id
}

/// Names the organizer of raffle `id`, as registration needs.
async fn name_organizer(app: &AppState, actor: service::Actor<'_>, id: &str) {
    let mut settings = service::settings_of(&raffles::get(&app.db, id).await.unwrap().unwrap());
    settings.organizer_name = "Cloud Native Budapest".into();
    settings.contact_email = "hello@example.com".into();
    service::update_settings(app, actor, id, settings)
        .await
        .unwrap();
}

async fn setup_open(app: &AppState, prizes: &[(&str, u32)]) -> RaffleRow {
    let id = create(app, OWNER, "Cloud Native Budapest").await;
    for (i, (name, qty)) in prizes.iter().enumerate() {
        service::upsert_prize(
            app,
            ACTOR,
            &id,
            PrizeInput {
                name: (*name).into(),
                qty: *qty,
                position: i as i64,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    service::lifecycle(app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    service::lifecycle(app, ACTOR, &id, LifecycleCommand::Open)
        .await
        .unwrap();
    raffles::get(&app.db, &id).await.unwrap().unwrap()
}

/// The raffle as the admin and the projector read it, the raffle as drawn
/// included.
async fn full(app: &AppState, id: &str) -> Full {
    Full::read(&app.db, id).await.unwrap().unwrap()
}

/// Waits up to `within` until the raffle is as `f` wants.
async fn wait_within<F: Fn(&Full) -> bool>(
    app: &AppState,
    id: &str,
    within: Duration,
    f: F,
) -> Full {
    let step = Duration::from_millis(20);
    for _ in 0..within.as_millis() / step.as_millis() {
        let r = full(app, id).await;
        if f(&r) {
            return r;
        }
        tokio::time::sleep(step).await;
    }
    panic!("condition not met in {within:?}");
}

async fn wait_for<F: Fn(&Full) -> bool>(app: &AppState, id: &str, f: F) -> Full {
    wait_within(app, id, Duration::from_secs(4), f).await
}

/// [`wait_for`] for slow conditions (stalls take a round period plus the timeout).
async fn wait_long<F: Fn(&Full) -> bool>(app: &AppState, id: &str, f: F) -> Full {
    wait_within(app, id, Duration::from_secs(20), f).await
}

/// An app whose drand relay is unreachable: every draw stalls on its round
/// (quicknet, 3 s lead, gives up 1 s after the round is due).
async fn unreachable_drand_app() -> AppState {
    let mut cfg = Config::for_tests();
    cfg.drand_chain = rafi_raffle::drand::chains::QUICKNET_HASH.to_string();
    let drand = rafi_engine::drand::DrandConfig {
        relays: vec!["http://127.0.0.1:9".into()],
        mock: false,
        lead_secs: 3,
        timeout_secs: 1,
    };
    app_with_engine(cfg, drand, LiveUpdates::polling()).await
}

#[tokio::test]
async fn full_raffle_flow() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(
        &app,
        &[("KCD Budapest ticket", 2), ("CNCF exam voucher", 1)],
    )
    .await;
    let code = raffle.join_code.clone();
    assert_eq!(code.len(), service::JOIN_CODE_LEN);
    assert!(code.bytes().all(|b| service::codes::ALPHABET.contains(&b)));

    // The join page renders the form.
    let (status, _, page) = send(
        &router,
        Request::get(format!("/j/{code}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("name=\"email\""));
    // The page ends with the loader of the participant build this server
    // was compiled against, which links the build's hashed glue and its
    // fallback under /participant/, where build_router serves them.
    assert!(page.contains(super::pages::LOADER), "{page}");
    assert!(
        super::pages::LOADER.contains("import(\"/participant/assets/rafi-participant-dxh")
            && super::pages::LOADER.contains("\"/participant/fallback.js\""),
        "{}",
        super::pages::LOADER
    );
    // Typed as the projector shows it, in groups and in any case: the
    // canonical code is redirected to.
    let grouped = format!("{}-{}", &code[..3], code[3..].to_lowercase());
    let (status, headers, _) = send(
        &router,
        Request::get(format!("/j/{grouped}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(headers[header::LOCATION], format!("/j/{code}"));

    // Participants join; the cookie brings them back to their entry.
    let mut cookies = Vec::new();
    for i in 0..5 {
        let (status, cookie, _) = join(
            &router,
            &code,
            &format!("Person {i}"),
            &format!("p{i}@example.com"),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "join {i}");
        cookies.push(cookie.expect("entrant cookie"));
    }
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{code}"))
            .header(header::COOKIE, &cookies[0])
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains("Person 0"));
    let me = entrants::list(&app.db, &raffle.id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.display_name == "Person 0")
        .unwrap();
    assert!(
        page.contains(me.claim_code.as_deref().unwrap()),
        "claim code is shown to the entrant"
    );

    // Same email (normalized) on another device: already in, no new entry, no cookie.
    let (status, cookie, page) = join(&router, &code, "Someone", "P0+other@Example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_none());
    assert!(page.contains("already entered"));

    // A person without a phone.
    service::add_manual(&app, ACTOR, &raffle.id, "No Phone", None)
        .await
        .unwrap();
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 6);

    // Freeze; joining is closed afterwards.
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    assert_eq!(r.raffle.state, RaffleState::Frozen);
    let (status, cookie, _) = join(&router, &code, "Late", "late@example.com").await;
    assert_ne!(status, StatusCode::SEE_OTHER);
    assert!(cookie.is_none());
    assert!(
        service::add_manual(&app, ACTOR, &raffle.id, "Late", None)
            .await
            .is_err()
    );
    assert!(
        service::upsert_prize(
            &app,
            ACTOR,
            &raffle.id,
            PrizeInput {
                name: "x".into(),
                qty: 1,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );

    // Draw every item; the first winner is not here and the item is redrawn.
    let state = r.drawn.as_ref().unwrap();
    let items: Vec<_> = state.items().iter().map(|u| u.id.clone()).collect();
    let mut winners = Vec::new();
    for (i, item) in items.iter().enumerate() {
        service::command(
            &app,
            ACTOR,
            &raffle.id,
            DrawRequest { item: item.clone() },
            &format!("c{i}"),
        )
        .await
        .unwrap();
        let r = wait_for(&app, &raffle.id, |r| {
            r.drawn
                .as_ref()
                .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Drawn)
        })
        .await;
        if i == 0 {
            let first = r
                .drawn
                .as_ref()
                .unwrap()
                .item(item)
                .unwrap()
                .winner()
                .cloned()
                .unwrap();
            service::command(
                &app,
                ACTOR,
                &raffle.id,
                Decision::Resolve {
                    item: item.clone(),
                    resolution: Resolution::NotHere,
                },
                "nh",
            )
            .await
            .unwrap();
            service::command(
                &app,
                ACTOR,
                &raffle.id,
                DrawRequest { item: item.clone() },
                "redraw",
            )
            .await
            .unwrap();
            let r = wait_for(&app, &raffle.id, |r| {
                r.drawn
                    .as_ref()
                    .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Drawn)
            })
            .await;
            assert_ne!(
                r.drawn
                    .as_ref()
                    .unwrap()
                    .item(item)
                    .unwrap()
                    .winner()
                    .cloned()
                    .unwrap(),
                first
            );
        }
        let r = full(&app, &raffle.id).await;
        winners.push(
            r.drawn
                .as_ref()
                .unwrap()
                .item(item)
                .unwrap()
                .winner()
                .cloned()
                .unwrap(),
        );
        service::command(
            &app,
            ACTOR,
            &raffle.id,
            Decision::Resolve {
                item: item.clone(),
                resolution: Resolution::Claimed,
            },
            &format!("r{i}"),
        )
        .await
        .unwrap();
    }
    let mut unique = winners.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), winners.len(), "one win per person");

    // Drawing a claimed item again is rejected.
    assert!(
        service::command(
            &app,
            ACTOR,
            &raffle.id,
            DrawRequest {
                item: items[0].clone(),
            },
            "x"
        )
        .await
        .is_err()
    );

    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Finished);
    assert!(r.finished_at.is_some());

    // The audit log names each command by its verb, with what it was about.
    let audit: Vec<(String, String)> = app
        .db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(&raffle.id)
        .all()
        .await
        .unwrap();
    let actions: Vec<&str> = audit.iter().map(|(a, _)| a.as_str()).collect();
    assert!(actions.contains(&"freeze"), "{audit:?}");
    assert!(!actions.contains(&"command"), "{audit:?}");
    assert!(
        audit.contains(&("draw".into(), items[0].to_string())),
        "{audit:?}"
    );
    assert!(
        audit.contains(&("resolve".into(), format!("{} not_here", items[0]))),
        "{audit:?}"
    );
    assert!(
        audit.contains(&("resolve".into(), format!("{} claimed", items[0]))),
        "{audit:?}"
    );
    assert_eq!(audit.last(), Some(&("finish".into(), String::new())));

    // The public log verifies independently.
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let report = rafi_raffle::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_raffle::drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), items.len() + 1);

    // CSV export lists the winners (and the not-here entrant).
    let csv = service::export_csv(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(csv.lines().count(), 1 + items.len() + 1);

    // Purging: personal data is erased but the draw stays verifiable.
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(entrants.len(), 6);
    assert!(
        entrants
            .iter()
            .all(|e| e.erased && e.email_raw.is_none() && e.token_hash.is_none())
    );
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(
        rafi_raffle::verify(
            &data.manifest,
            &data.manifest_hash,
            &data.events,
            rafi_raffle::drand::resolve_chain
        )
        .ok()
    );
    assert!(data.names.is_empty(), "no names are left: {:?}", data.names);
    assert!(entrants.iter().all(|e| e.claim_code.is_none()));
    // The join code no longer answers (nobody can be recognized on it, and
    // guessing codes finds only raffles inside their hold); the slug does.
    let (status, _, _) = send(
        &router,
        Request::get(format!("/j/{code}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a purged raffle's code");
    let (status, _, _) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the draw stays verifiable");
}

#[tokio::test]
async fn public_view_has_no_personal_data() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    join(
        &router,
        &raffle.join_code,
        "Visible",
        "secret-address@example.com",
    )
    .await;
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let state = full(&app, &raffle.id).await.drawn.unwrap();
    service::command(
        &app,
        ACTOR,
        &raffle.id,
        DrawRequest {
            item: state.items()[0].id.clone(),
        },
        "c",
    )
    .await
    .unwrap();
    wait_for(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.items()[0].status() == ItemStatus::Drawn)
    })
    .await;

    let (status, _, body) = send(
        &router,
        Request::get(format!("/api/r/{}/view", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let claim = entrants::list(&app.db, &raffle.id).await.unwrap()[0]
        .claim_code
        .clone()
        .unwrap();
    assert!(body.contains("Visible"));
    assert!(!body.contains("secret-address"), "email leaked: {body}");
    assert!(!body.contains(&format!("\"{claim}\"")), "claim code leaked");
    let (_, headers, verify) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(!verify.contains("secret-address"));

    // verify.json names its format and version; the version is its ETag, and
    // a reader revalidates rather than keeps a copy (a name erased later
    // must never come from a cache).
    let data: rafi_api::VerifyData = serde_json::from_str(&verify).unwrap();
    assert_eq!(data.format, rafi_api::VERIFY_FORMAT);
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    assert_eq!(etag, format!("\"v{}\"", data.v));
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let (status, headers, body) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!((status, body.as_str()), (StatusCode::NOT_MODIFIED, ""));
    assert_eq!(headers[header::ETAG], etag);
    // Erasing a name moves the version, so the ETag changes with the content.
    let hidden = &entrants::list(&app.db, &raffle.id).await.unwrap()[0];
    service::remove_entrant(&app, ACTOR, &raffle.id, &hidden.id)
        .await
        .unwrap_err();
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &hidden.id,
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (status, headers, body) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .header(header::IF_NONE_MATCH, &etag)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(headers[header::ETAG], etag);
    assert!(
        !body.contains("Visible"),
        "the hidden name is masked: {body}"
    );
}

async fn get_json(
    router: &Router,
    url: &str,
) -> (StatusCode, header::HeaderMap, serde_json::Value) {
    let (status, headers, body) =
        send(router, Request::get(url).body(Body::empty()).unwrap()).await;
    let json = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    (status, headers, json)
}

async fn version(app: &AppState, id: &str) -> u64 {
    raffles::get(&app.db, id).await.unwrap().unwrap().version as u64
}

/// The full view's `v` is the raffle's `version` column, which every write
/// that changes a screen bumps; the view still carries the server's clock.
#[tokio::test]
async fn full_view_v_is_the_version_column() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let url = format!("/api/r/{}/view", raffle.slug);

    let (status, headers, view) = get_json(&router, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    let v0 = view["v"].as_u64().unwrap();
    assert_eq!(v0, version(&app, &raffle.id).await);
    assert!(view["server_now_ms"].as_u64().unwrap() > 0);
    assert_eq!(
        view["follow"],
        serde_json::json!({"kind": "poll", "every_ms": 2500}),
        "no push natively: clients poll"
    );

    // Polling again without a write: the same version.
    let (_, _, again) = get_json(&router, &url).await;
    assert_eq!(again["v"].as_u64().unwrap(), v0);

    service::add_manual(&app, ACTOR, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let (_, _, view) = get_json(&router, &url).await;
    let v1 = view["v"].as_u64().unwrap();
    assert!(v1 > v0, "{v1} > {v0}");
    assert_eq!(v1, version(&app, &raffle.id).await);
    assert_eq!(view["entrant_count"], 1);

    // The admin's copy is the same view.
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.view.v, v1);
}

/// What phones poll: the participant view, with no personal data beyond the
/// names of drawn entrants (masked when hidden), never cached.
#[tokio::test]
async fn participant_view_has_no_personal_data() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1), ("Other", 1)]).await;
    for (name, email) in [("Anna", "anna@example.com"), ("Bence", "bence@example.com")] {
        join(&router, &raffle.join_code, name, email).await;
    }
    let url = format!("/api/r/{}/participant", raffle.slug);
    let (status, headers, view) = get_json(&router, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    assert_eq!(view["state"], "open");
    assert_eq!(view["prizes_left"], 2);
    assert_eq!(view["v"].as_u64().unwrap(), version(&app, &raffle.id).await);
    let text = view.to_string();
    for secret in ["Anna", "Bence", "@example.com"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }

    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let item = r.drawn.as_ref().unwrap().items()[0].id.clone();
    service::command(
        &app,
        ACTOR,
        &raffle.id,
        DrawRequest { item: item.clone() },
        "c",
    )
    .await
    .unwrap();
    let r = wait_for(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&item).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    let winner_id = r
        .drawn
        .as_ref()
        .unwrap()
        .item(&item)
        .unwrap()
        .winner()
        .cloned()
        .unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    let winner = entrants
        .iter()
        .find(|e| e.id == winner_id.as_str())
        .unwrap();
    let other = entrants
        .iter()
        .find(|e| e.id != winner_id.as_str())
        .unwrap();

    let (_, _, view) = get_json(&router, &url).await;
    let text = view.to_string();
    assert_eq!(view["last"]["display_name"], winner.display_name.as_str());
    assert_eq!(view["winners"][0]["entrant_id"], winner_id.as_str());
    assert!(
        !text.contains(&other.display_name),
        "a non-winner's name: {text}"
    );
    assert!(!text.contains("@example.com"), "an email: {text}");
    for e in &entrants {
        assert!(
            !text.contains(&format!("\"{}\"", e.claim_code.as_deref().unwrap())),
            "a claim code: {text}"
        );
    }

    // A hidden winner's name is masked.
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        winner_id.as_str(),
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, _, view) = get_json(&router, &url).await;
    assert!(!view.to_string().contains(&winner.display_name));

    // The participant page embeds the same view and renders its card.
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains("id=\"rafi-config\""));
    assert!(page.contains(&format!("\"participant_url\":\"{url}\"")));
    assert!(page.contains(r#""follow":{"kind":"poll","every_ms":2500}"#));
    assert!(
        !page.contains(&other.display_name),
        "a non-winner's name on the page"
    );
}

#[tokio::test]
async fn drafts_are_private() {
    let app = app().await;
    let router = routes(app.clone());
    let id = create(&app, OWNER, "Secret").await;
    let raffle = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let (status, _, _) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    for path in ["view", "participant"] {
        let (status, _, _) = send(
            &router,
            Request::get(format!("/api/r/{}/{path}", raffle.slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }
    // Other organizers cannot see it either.
    assert!(matches!(
        service::admin(&app, "someone_else", &id).await,
        Err(service::Error::NotFound)
    ));
}

#[tokio::test]
async fn join_requires_proof_of_work_and_consent() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let post = |body: String| {
        Request::post(format!("/j/{}", raffle.join_code))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap()
    };
    let (status, _, page) = send(
        &router,
        post(form(&[
            ("name", "Bot"),
            ("email", "bot@example.com"),
            ("rules", "on"),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("human"));
    let proof = solved_pow(&router).await;
    let (status, _, page) = send(
        &router,
        post(form(&[
            ("name", "Ann"),
            ("email", "ann@example.com"),
            ("proof", &proof),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("accept the rules"));
    let proof = solved_pow(&router).await;
    let (status, _, _) = send(
        &router,
        post(form(&[
            ("name", "Porn King"),
            ("email", "x@example.com"),
            ("rules", "on"),
            ("proof", &proof),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The app decides the chain raffles commit to: an engine on another chain
/// is refused before anything is frozen.
#[tokio::test]
async fn an_engine_on_another_chain_is_refused_before_freezing() {
    let mut cfg = Config::for_tests();
    cfg.drand_chain = rafi_raffle::drand::chains::QUICKNET_HASH.to_string();
    let app = app_with_engine(cfg, mock_drand(), LiveUpdates::polling()).await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    service::add_manual(&app, ACTOR, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let err = service::start_draw(&app, ACTOR, &raffle.id)
        .await
        .unwrap_err();
    assert!(matches!(err, service::Error::Refused(_)), "{err}");
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Open, "nothing was frozen");
    assert!(r.manifest.is_none(), "nothing was committed");
}

/// Without a raffle engine configured, nothing can start.
#[tokio::test]
async fn draws_need_restate() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = AppState::new(cfg).await.unwrap();
    assert!(app.engine.is_none());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    service::add_manual(&app, ACTOR, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let err = service::start_draw(&app, ACTOR, &raffle.id)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Restate"), "{err}");
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Open,
        "nothing was frozen"
    );
}

/// POSTs `args` (JSON) to the admin function `name`.
fn admin_call(name: &str, args: &str) -> Request<Body> {
    Request::post(format!("/api/admin/{name}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(args.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn admin_requires_clerk_session() {
    use dioxus_clerk::testing::TestClerk;
    let app = app().await;
    let clerk = TestClerk::new().unwrap();
    // A stand-in for the web bundle.
    let public = std::env::temp_dir().join(format!("rafi-public-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&public).unwrap();
    std::fs::write(
        public.join("index.html"),
        "<!doctype html><html><head></head><body><div id=\"main\"></div></body></html>",
    )
    .unwrap();
    // Its files are under assets/.
    std::fs::create_dir_all(public.join("assets")).unwrap();
    std::fs::write(public.join("assets/rafi-web.js"), "// web").unwrap();
    // And for the participant build: a hashed asset and the fallback, as
    // the build leaves them, a precompressed variant next to the asset,
    // and the loader this server compiled in.
    let participant = public.join("participant-dist");
    std::fs::create_dir_all(participant.join("assets")).unwrap();
    let glue = "assets/rafi-participant-dxh0123456789abcdef.js";
    std::fs::write(participant.join(glue), "// glue").unwrap();
    std::fs::write(participant.join(format!("{glue}.gz")), "gzipped bytes").unwrap();
    std::fs::write(participant.join("fallback.js"), "// fallback").unwrap();
    std::fs::write(participant.join("index.html"), super::pages::LOADER).unwrap();
    let dirs = super::Dirs {
        public: public.clone(),
        participant,
    };
    // The directory holds the build this server was compiled against;
    // another build's loader (other names) is refused.
    dirs.check_participant().unwrap();
    std::fs::write(
        dirs.participant.join("index.html"),
        "<script>other</script>",
    )
    .unwrap();
    let err = dirs.check_participant().unwrap_err().to_string();
    assert!(err.contains("not the participant build"), "{err}");
    // Clerk's layer as both hosts build it: the public origin is the one
    // authorized party.
    let router = super::build_router(
        app.clone(),
        Some(super::auth::clerk_layer(&app.cfg, clerk.config().unwrap()).unwrap()),
        &dirs,
        dirs.shell().unwrap(),
    );
    let get = |path: &str| Request::get(path).body(Body::empty()).unwrap();
    let asset = format!("/participant/{glue}");
    let (status, headers, body) = send(&router, get(&asset)).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "// glue"));
    // Named by its hash, so a phone keeps it for good; still with an ETag,
    // and a 304 for a phone that revalidates anyway.
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let etag = headers[header::ETAG].clone();
    let mut req = get(&asset);
    req.headers_mut().insert(header::IF_NONE_MATCH, etag);
    let (status, headers, body) = send(&router, req).await;
    assert_eq!((status, body.as_str()), (StatusCode::NOT_MODIFIED, ""));
    assert_eq!(
        headers[header::CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let mut req = get(&asset);
    req.headers_mut()
        .insert(header::ACCEPT_ENCODING, "gzip, br".parse().unwrap());
    let (status, headers, body) = send(&router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
    assert_eq!(body, "gzipped bytes", "the precompressed file is served");
    // Another build's name (a page rendered before a deploy) is a plain 404:
    // nothing a browser would keep.
    let (status, headers, _) = send(
        &router,
        get("/participant/assets/rafi-participant-dxhfedcba9876543210.js"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(headers.get(header::CACHE_CONTROL).is_none(), "{headers:?}");
    // The fallback keeps its name, so phones revalidate it.
    let (status, headers, body) = send(&router, get("/participant/fallback.js")).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "// fallback"));
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert!(headers.contains_key(header::ETAG));

    let (status, headers, body) = send(&router, admin_call("whoami", "{}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("Please sign in."), "{body}");
    // Behind Clerk's layer, every response has an ID, refusals included.
    let id = headers[super::trace::REQUEST_ID].to_str().unwrap();
    assert_eq!(id.len(), 16, "{id}");

    let mut req = admin_call("whoami", "{}");
    req.headers_mut().insert(
        header::COOKIE,
        clerk.cookie("user_2abc").unwrap().parse().unwrap(),
    );
    let (status, _, body) = send(&router, req).await;
    assert!(status.is_success(), "{status}: {body}");
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(me.owner_id, "user_2abc");
    assert!(!me.dev_auth);

    // A token minted for this origin passes; one minted for another site on
    // the same Clerk instance doesn't (`azp`).
    let for_origin = |party: &str| {
        let mut req = admin_call("whoami", "{}");
        req.headers_mut().insert(
            header::COOKIE,
            clerk
                .cookie_for(&clerk.session("user_2abc").with_authorized_party(party))
                .unwrap()
                .parse()
                .unwrap(),
        );
        req
    };
    let (status, _, body) = send(&router, for_origin(&app.cfg.public_url)).await;
    assert!(status.is_success(), "our origin: {status}: {body}");
    let (status, _, body) = send(&router, for_origin("https://other.example")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "another origin: {body}");

    // The arguments must be JSON: what a cross-site form could send is refused.
    let mut req = Request::post("/api/admin/create_raffle")
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(
            r#"{"title":"x","event_date":"2026-10-28","event_ends_at":null}"#,
        ))
        .unwrap();
    req.headers_mut().insert(
        header::COOKIE,
        clerk.cookie("user_2abc").unwrap().parse().unwrap(),
    );
    let (status, _, body) = send(&router, req).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{body}");
    assert!(
        service::list(&app, "user_2abc").await.unwrap().is_empty(),
        "nothing was created"
    );

    // The web app, with a single-page fallback for its own routes, and the
    // security headers: the admin can't be framed, the projector and the
    // verification page can.
    for path in [
        "/",
        "/admin",
        "/admin/r/abc",
        "/s/some-slug",
        "/v/some-slug",
    ] {
        let (status, headers, body) =
            send(&router, Request::get(path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains("<div id=\"main\">"), "{path}: {body}");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
        assert_eq!(
            headers[header::REFERRER_POLICY],
            "strict-origin-when-cross-origin",
            "{path}"
        );
        let framed = path.starts_with("/s/") || path.starts_with("/v/");
        // The web app's policy (`csp_by_page` has the rest), framing denied
        // but where it may be framed.
        let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(policy.starts_with("default-src 'self'"), "{path}: {policy}");
        assert_eq!(
            policy.contains("frame-ancestors 'none'"),
            !framed,
            "{path}: {policy}"
        );
        assert_eq!(
            headers.contains_key(header::X_FRAME_OPTIONS),
            !framed,
            "{path}"
        );
        assert!(headers.contains_key(super::trace::REQUEST_ID), "{path}");
    }
    // The web app's files, natively from its directory, with the headers
    // too; a miss there is a 404, not the shell.
    let (status, headers, body) = send(&router, get("/assets/rafi-web.js")).await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "// web"));
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let (status, _, _) = send(&router, get("/assets/missing.js")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The shell at its own path too, under its policy.
    let (status, headers, body) = send(&router, get("/index.html")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<div id=\"main\">"), "{body}");
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("'unsafe-eval'")
    );
    // The participant build's files carry the headers too.
    let (_, headers, _) = send(&router, get("/participant/fallback.js")).await;
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    let (_, headers, _) = send(&router, get(&asset)).await;
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    // And on what the server renders itself.
    let (_, headers, _) = send(
        &router,
        Request::get("/healthz").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(
        headers[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    std::fs::remove_dir_all(&public).ok();
}

/// Every page carries the Content-Security-Policy of its kind (`csp`): the
/// server's pages allow their inline scripts by hash and nothing else
/// inline, the installation's documents also images from elsewhere, the web
/// app's shell eval, inline styles and Clerk; anything else only denies
/// framing. The router as the Worker composes it.
#[tokio::test]
async fn csp_by_page() {
    use super::{Shell, config::CspMode, csp, shell_routes};
    const SHELL: &str = "<!doctype html><html><head><script src=\"/config.js\"></script></head><body><div id=\"main\"></div></body></html>";
    let router_of = |app: &AppState| {
        routes(app.clone()).fallback_service(shell_routes(app, Some(Shell::new(SHELL).unwrap())))
    };
    let get = |path: &str| Request::get(path).body(Body::empty()).unwrap();
    let csp_of = |headers: &header::HeaderMap, name: header::HeaderName| {
        headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default()
    };
    let policy = |headers: &header::HeaderMap| csp_of(headers, header::CONTENT_SECURITY_POLICY);
    let directive = |policy: &str, name: &str| -> Vec<String> {
        policy
            .split("; ")
            .find_map(|d| d.strip_prefix(name)?.strip_prefix(' '))
            .map(|sources| sources.split(' ').map(str::to_owned).collect())
            .unwrap_or_default()
    };
    let app = app().await;
    let router = router_of(&app);
    let raffle = setup_open(&app, &[("Prize", 1)]).await;

    // The server's pages: what they carry inline, by hash; no eval, no
    // inline styles, no Clerk; never framed.
    let join = format!("/j/{}", raffle.join_code);
    for path in [
        join.clone(),
        format!("/rules/{}", raffle.slug),
        format!("/privacy/{}", raffle.slug),
        "/j/no-such-code".to_owned(),
    ] {
        let (_, headers, page) = send(&router, get(&path)).await;
        let p = policy(&headers);
        let scripts = directive(&p, "script-src");
        let inline = csp::inline_scripts(&page);
        assert!(!inline.is_empty(), "{path}: the head script at least");
        for script in inline {
            assert!(
                scripts.contains(&csp::hash_source(script)),
                "{path}: {script:?} isn't allowed by {p}"
            );
        }
        for loose in ["'unsafe-eval'", "'unsafe-inline'"] {
            assert!(!p.contains(loose), "{path}: {p}");
        }
        assert_eq!(directive(&p, "style-src"), ["'self'"], "{path}");
        assert_eq!(directive(&p, "img-src"), ["'self'", "data:"], "{path}");
        assert_eq!(
            directive(&p, "connect-src"),
            ["'self'", "ws://localhost:8080"],
            "{path}: the live channel's socket, named"
        );
        assert_eq!(directive(&p, "form-action"), ["'self'"], "{path}");
        assert_eq!(directive(&p, "frame-ancestors"), ["'none'"], "{path}");
        assert!(!p.contains("clerk"), "{path}: {p}");
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY", "{path}");
    }
    // The join page runs the participant build's loader, inline too.
    let (_, _, page) = send(&router, get(&join)).await;
    assert_eq!(
        csp::inline_scripts(&page).len(),
        2,
        "the head script and the loader"
    );

    // The installation's documents: the same, and images from any https:
    // origin, which the operator's Markdown may show.
    let (_, headers, _) = send(&router, get(&format!("/rules/{}", raffle.slug))).await;
    let server_page = policy(&headers);
    for path in ["/privacy", "/terms"] {
        let (status, headers, _) = send(&router, get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        let p = policy(&headers);
        assert_eq!(
            directive(&p, "img-src"),
            ["'self'", "data:", "https:"],
            "{path}"
        );
        assert_eq!(
            directive(&p, "script-src"),
            directive(&server_page, "script-src"),
            "{path}"
        );
    }

    // The web app's shell, at every path of its own: eval (Dioxus), inline
    // styles, and Clerk's host for the publishable key, with its
    // protection's; no inline scripts.
    for path in ["/", "/admin", "/admin/r/abc", "/index.html"] {
        let (status, headers, body) = send(&router, get(path)).await;
        assert_eq!((status, body.as_str()), (StatusCode::OK, SHELL), "{path}");
        let p = policy(&headers);
        let scripts = directive(&p, "script-src");
        for source in [
            "'self'",
            "'wasm-unsafe-eval'",
            "'unsafe-eval'",
            "https://rafi-test.clerk.accounts.dev",
            "https://challenges.cloudflare.com",
        ] {
            assert!(
                scripts.contains(&source.to_owned()),
                "{path}: {source} in {p}"
            );
        }
        assert!(!p.contains("'sha256-"), "{path}: no inline scripts: {p}");
        assert!(
            directive(&p, "connect-src")
                .contains(&"https://rafi-test.clerk.accounts.dev".to_owned()),
            "{p}"
        );
        assert!(
            directive(&p, "style-src").contains(&"'unsafe-inline'".to_owned()),
            "{p}"
        );
        assert_eq!(directive(&p, "frame-ancestors"), ["'none'"], "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-cache", "{path}");
    }
    // Revalidated by its entity tag.
    let (_, headers, _) = send(&router, get("/admin")).await;
    let mut req = get("/admin");
    req.headers_mut()
        .insert(header::IF_NONE_MATCH, headers[header::ETAG].clone());
    let (status, _, body) = send(&router, req).await;
    assert_eq!((status, body.as_str()), (StatusCode::NOT_MODIFIED, ""));
    // The projector and the verification page may be framed.
    for path in ["/s/some-slug", "/v/some-slug"] {
        let (_, headers, _) = send(&router, get(path)).await;
        let p = policy(&headers);
        assert!(p.starts_with("default-src 'self'"), "{path}: {p}");
        assert!(!p.contains("frame-ancestors"), "{path}: {p}");
        assert!(!headers.contains_key(header::X_FRAME_OPTIONS), "{path}");
    }

    // What isn't a page only denies framing; a miss among the files and APIs
    // is a 404, not the shell, and the shell is for reading.
    let (_, headers, _) = send(&router, get("/healthz")).await;
    assert_eq!(policy(&headers), csp::NO_FRAMING);
    for path in [
        "/assets/rafi-web-dxh0.js",
        "/participant/x.js",
        "/api/nothing",
        "/legal/terms.md",
    ] {
        let (status, headers, body) = send(&router, get(path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(policy(&headers), csp::NO_FRAMING, "{path}");
    }
    let (status, _, _) = send(
        &router,
        Request::post("/admin").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    // Dev auth loads no Clerk, so the policy names none of it.
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let dev = app_with(cfg).await;
    let (_, headers, _) = send(&router_of(&dev), get("/admin")).await;
    let p = policy(&headers);
    assert!(!p.contains("clerk") && !p.contains("cloudflare"), "{p}");
    assert_eq!(directive(&p, "worker-src"), ["'self'"]);

    // RAFI_CSP=report-only: every page's policy is reported, not enforced;
    // framing stays denied.
    let mut cfg = Config::for_tests();
    cfg.csp = CspMode::ReportOnly;
    let reporting = app_with(cfg).await;
    let router = router_of(&reporting);
    for path in ["/admin", "/terms", "/j/no-such-code"] {
        let (_, headers, _) = send(&router, get(path)).await;
        assert_eq!(policy(&headers), csp::NO_FRAMING, "{path}");
        let reported = csp_of(&headers, header::CONTENT_SECURITY_POLICY_REPORT_ONLY);
        assert!(
            reported.starts_with("default-src 'self'"),
            "{path}: {reported}"
        );
    }
    let (_, headers, _) = send(&router, get("/s/some-slug")).await;
    assert!(!headers.contains_key(header::CONTENT_SECURITY_POLICY));
    assert!(headers.contains_key(header::CONTENT_SECURITY_POLICY_REPORT_ONLY));

    // A shell with an inline script would load and not run it: refused.
    let err = Shell::new("<head><script>alert(1)</script></head>").unwrap_err();
    assert!(err.to_string().contains("inline script"), "{err}");
}

#[tokio::test]
async fn pow_worker_is_dioxus_altchas() {
    let app = app().await;
    let public = std::env::temp_dir().join(format!("rafi-public-{}", uuid::Uuid::new_v4()));
    let participant = public.join("participant-dist");
    std::fs::create_dir_all(&participant).unwrap();
    // A participant directory built before the route: its copy is ignored.
    std::fs::write(participant.join("pow-worker.js"), "// stale").unwrap();
    let router = super::build_router(
        app,
        None,
        &super::Dirs {
            public: public.clone(),
            participant,
        },
        None,
    );
    let get = || {
        Request::get("/participant/pow-worker.js")
            .body(Body::empty())
            .unwrap()
    };

    let (status, headers, body) = send(&router, get()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, dioxus_altcha::WORKER_JS);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();

    // Revalidating: unchanged, so no body.
    for if_none_match in [
        etag.clone(),
        format!("W/{etag}"),
        format!("\"x\", {etag}"),
        "*".into(),
    ] {
        let mut req = get();
        req.headers_mut()
            .insert(header::IF_NONE_MATCH, if_none_match.parse().unwrap());
        let (status, headers, body) = send(&router, req).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::NOT_MODIFIED, ""),
            "{if_none_match}"
        );
        assert_eq!(headers[header::ETAG], etag.as_str());
    }
    let mut req = get();
    req.headers_mut()
        .insert(header::IF_NONE_MATCH, "\"other\"".parse().unwrap());
    let (status, _, body) = send(&router, req).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, dioxus_altcha::WORKER_JS)
    );
    std::fs::remove_dir_all(&public).ok();
}

/// `/config.js` tells the web app how the server signs organizers in: the
/// Clerk publishable key when it verifies Clerk sessions, none under dev
/// auth. The server is the one authority, so the web app can't disagree
/// with it, and one web app bundle serves every installation.
#[tokio::test]
async fn config_js_says_how_the_server_signs_in() {
    // And it applies the saved theme before first paint, which the web app's
    // shell can't do inline under its policy.
    const THEME: &str = r#"try{var t=localStorage.getItem("rafi-theme");if(t)document.documentElement.setAttribute("data-theme",t)}catch(e){}"#;
    let get = || Request::get("/config.js").body(Body::empty()).unwrap();

    let router = routes(app().await);
    let (status, headers, body) = send(&router, get()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    // A rotated key or a switched mode takes effect on the next load.
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(
        body,
        format!(
            "window.__RAFI_CONFIG__ = {{\"clerkPublishableKey\":{:?}}};\n{THEME}\n",
            Config::for_tests().clerk_publishable_key.unwrap()
        )
    );

    // Dev auth: no key, whatever is configured, so the admin shows its
    // dev-auth badge instead of a sign-in nobody checks.
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let router = routes(app_with(cfg).await);
    let (status, _, body) = send(&router, get()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        format!("window.__RAFI_CONFIG__ = {{\"clerkPublishableKey\":null}};\n{THEME}\n")
    );
}

#[tokio::test]
async fn admin_api_needs_a_session_unless_dev_auth() {
    // No session and no dev auth: every admin function is refused.
    let router = routes(app().await);
    for name in [
        "whoami",
        "list_raffles",
        "create_raffle",
        "no_such_function",
    ] {
        let (status, _, body) = send(&router, admin_call(name, r#"{"title":"x"}"#)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{name}: {body}");
        let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
        assert_eq!(
            (err.code.as_str(), err.message.as_str()),
            (rafi_api::admin::codes::UNAUTHORIZED, "Please sign in.")
        );
    }

    // Dev auth: the organizer is `dev`.
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = AppState::new(cfg).await.unwrap();
    let router = routes(app.clone());

    let (status, _, body) = send(&router, admin_call("whoami", "")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!((me.owner_id.as_str(), me.dev_auth), ("dev", true));

    let create = r#"{"title":"Meetup #1","event_date":"2026-10-28","event_ends_at":null}"#;
    let (status, _, body) = send(&router, admin_call("create_raffle", create)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id: String = serde_json::from_str(&body).unwrap();

    let (status, _, body) = send(&router, admin_call("list_raffles", "{}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: Vec<rafi_api::RaffleSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "Meetup #1");

    // Unit results are JSON `null`.
    let (status, _, body) = send(
        &router,
        admin_call(
            "set_lifecycle",
            &format!(r#"{{"id":"{id}","action":"publish"}}"#),
        ),
    )
    .await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "null"));
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Ready
    );

    // Errors carry the service's message and a matching status.
    let (status, _, body) = send(
        &router,
        admin_call(
            "create_raffle",
            r#"{"title":" ","event_date":"2026-10-28","event_ends_at":null}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (err.code.as_str(), err.message.as_str()),
        ("title", "The title must be 1–120 characters."),
        "a refusal's code is its kind"
    );
    let (status, _, body) = send(
        &router,
        admin_call(
            "create_raffle",
            r#"{"title":"Meetup","event_date":"soon","event_ends_at":null}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("calendar day"), "{body}");
    let code = |body: &str| {
        serde_json::from_str::<rafi_api::admin::ApiError>(body)
            .unwrap()
            .code
    };
    let (status, headers, body) = send(&router, admin_call("get_raffle", r#"{"id":"nope"}"#)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(code(&body), rafi_api::admin::codes::NOT_FOUND);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store", "errors too");
    let (status, _, body) = send(&router, admin_call("get_raffle", "{}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(code(&body), rafi_api::admin::codes::INVALID_ARGUMENTS);
    let (status, _, body) = send(&router, admin_call("no_such_function", "{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(code(&body), rafi_api::admin::codes::NOT_FOUND);
}

#[tokio::test]
async fn lifecycle_guards() {
    let app = app().await;
    let id = create(&app, OWNER, "Guards").await;
    assert!(
        service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open)
            .await
            .is_err(),
        "cannot open a draft"
    );
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open)
        .await
        .unwrap();
    assert!(
        service::start_draw(&app, ACTOR, &id).await.is_err(),
        "no prizes"
    );
    service::upsert_prize(
        &app,
        ACTOR,
        &id,
        PrizeInput {
            name: "P".into(),
            qty: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        service::start_draw(&app, ACTOR, &id).await.is_err(),
        "no entrants"
    );
    // The failed freeze must not leave the raffle frozen.
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Open
    );
    assert!(
        service::add_fakes(&app, ACTOR, &id, 3).await.is_err(),
        "fakes need test mode"
    );
    let dup = service::duplicate(&app, ACTOR, &id, &service::EventDate::today(&app))
        .await
        .unwrap();
    assert_eq!(prizes::list(&app.db, &dup).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_freeze_interrupted_before_its_commit_can_still_start() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 2)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        service::add_manual(&app, ACTOR, &raffle.id, name, None)
            .await
            .unwrap();
    }
    // The freeze's first write landed, the manifest's did not.
    app.db
        .query("UPDATE raffles SET state = 'frozen' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();

    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let Full {
        raffle: r, drawn, ..
    } = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let doc: rafi_raffle::Manifest =
        serde_json::from_str(r.manifest.as_deref().expect("a manifest")).unwrap();
    assert_eq!(doc.entrants.len(), 3);
    assert_eq!(doc.items.len(), 2);
    assert_eq!(
        r.manifest_hash.as_deref(),
        Some(rafi_raffle::ManifestHash::of(r.manifest.as_deref().unwrap()).to_string()).as_deref()
    );
    assert!(r.frozen_at.is_some());
    assert_eq!(
        drawn.unwrap().manifest_hash().to_string(),
        r.manifest_hash.unwrap()
    );

    // Starting again re-sends the same commitment.
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let again = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(again.manifest, r.manifest);
}

/// The additional rules are fixed once someone accepted them: fakes don't
/// count, a person added by hand does, and the rest of the settings still
/// save.
#[tokio::test]
async fn rules_are_fixed_once_accepted() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    let save = |rules: &str, test_mode: bool| {
        let mut settings = service::settings_of(&raffle);
        settings.rules = rules.into();
        settings.test_mode = test_mode;
        service::update_settings(&app, ACTOR, &raffle.id, settings)
    };
    save("Rehearsal rules.", true).await.unwrap();
    service::add_fakes(&app, ACTOR, &raffle.id, 3)
        .await
        .unwrap();
    save("Fakes don't accept anything.", true).await.unwrap();
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(admin.actions.edit_rules);

    service::add_manual(&app, ACTOR, &raffle.id, "No Phone", None)
        .await
        .unwrap();
    let refused = save("Changed after someone accepted.", true).await;
    assert!(
        matches!(
            &refused,
            Err(service::Error::Refused(service::Refusal::SettingsFixed(fields)))
                if fields == &["additional rules"]
        ),
        "{refused:?}"
    );
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(!admin.actions.edit_rules);
    assert_eq!(admin.settings.rules, "Fakes don't accept anything.");
    // The event date, the period, the privacy text and link lock with them,
    // and the refusal names what changed.
    let mut settings = admin.settings.clone();
    settings.event_date = "2026-12-31".into();
    settings.retention_days = 1;
    settings.privacy_url = "https://example.com/other".into();
    let refused = service::update_settings(&app, ACTOR, &raffle.id, settings).await;
    assert!(
        matches!(
            &refused,
            Err(service::Error::Refused(service::Refusal::SettingsFixed(fields)))
                if fields == &["event date", "retention period", "privacy notice link"]
        ),
        "{refused:?}"
    );
    let before = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    // The same rules, other settings changed: saved. The privacy contact
    // stays editable: a contact must be current.
    let mut settings = admin.settings;
    settings.title = "Renamed".into();
    settings.privacy_email = "dpo@example.com".into();
    service::update_settings(&app, ACTOR, &raffle.id, settings)
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.title, "Renamed");
    assert_eq!(r.privacy_email, "dpo@example.com");
    assert_eq!(
        (r.event_date, r.erase_at, r.retention_days),
        (before.event_date, before.erase_at, before.retention_days),
        "the dates didn't move"
    );
}

/// A new raffle starts with a copy of its owner's defaults: changing them
/// later leaves it as it is.
#[tokio::test]
async fn new_raffles_start_with_the_owners_defaults() {
    let app = app().await;
    let defaults = rafi_api::RaffleDefaults {
        organizer_name: " Cloud Native Budapest ".into(),
        contact_email: "hello@example.com".into(),
        privacy_email: "dpo@example.com".into(),
        privacy_text: "We keep nothing else.".into(),
        privacy_url: "https://example.com/privacy".into(),
        rules: "Entrants must be 18 or older.".into(),
        retention_days: Some(3),
    };
    service::save_defaults(&app, ACTOR, defaults).await.unwrap();
    let saved = service::get_defaults(&app, OWNER).await.unwrap();
    assert_eq!(saved.organizer_name, "Cloud Native Budapest", "trimmed");

    let event = service::EventDate {
        date: "2026-10-28".into(),
        ends_at: None,
    };
    let id = service::create(&app, ACTOR, "Meetup", &event)
        .await
        .unwrap();
    let r = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert_eq!(
        (
            r.organizer_name.as_str(),
            r.contact_email.as_str(),
            r.privacy_email.as_str(),
            r.privacy_text.as_str(),
            r.privacy_url.as_str(),
            r.rules.as_str(),
            r.retention_days,
        ),
        (
            "Cloud Native Budapest",
            "hello@example.com",
            "dpo@example.com",
            "We keep nothing else.",
            "https://example.com/privacy",
            "Entrants must be 18 or older.",
            3,
        )
    );
    assert_eq!(r.event_date, "2026-10-28");
    assert_eq!(
        r.erase_at,
        r.event_ends_at + 3 * 86_400,
        "the erasure date follows the owner's period"
    );

    service::save_defaults(&app, ACTOR, Default::default())
        .await
        .unwrap();
    let r = raffles::get(&app.db, &id).await.unwrap().unwrap();
    assert_eq!(r.rules, "Entrants must be 18 or older.", "a copy");
    let other = service::create(&app, service::Actor::solo("someone-else"), "Theirs", &event)
        .await
        .unwrap();
    let r = raffles::get(&app.db, &other).await.unwrap().unwrap();
    assert!(r.organizer_name.is_empty() && r.rules.is_empty());
    assert_eq!(
        r.retention_days, app.policy.retention.days,
        "the installation's period without defaults"
    );

    // Defaults are checked like settings.
    let over = rafi_api::RaffleDefaults {
        retention_days: Some(app.policy.retention.max_days + 1),
        ..Default::default()
    };
    assert!(matches!(
        service::save_defaults(&app, ACTOR, over).await,
        Err(service::Error::Refused(service::Refusal::Retention { .. }))
    ));
}

#[tokio::test]
async fn fake_entrants_are_added_in_one_batch() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    let mut settings = service::settings_of(&raffle);
    settings.test_mode = true;
    service::update_settings(&app, ACTOR, &raffle.id, settings)
        .await
        .unwrap();
    service::add_fakes(
        &app,
        ACTOR,
        &raffle.id,
        rafi_lifecycle::entrant::MAX_FAKES + 1,
    )
    .await
    .unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(entrants.len(), rafi_lifecycle::entrant::MAX_FAKES as usize);
    assert!(
        entrants
            .iter()
            .all(|e| e.source == rafi_lifecycle::entrant::Source::Fake && !e.hidden)
    );
    let ids: std::collections::HashSet<_> = entrants.iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), entrants.len());
}

/// A draw whose round never arrives in time stalls on that round: retrying
/// keeps the round, and the only exit (abandon) is logged for the verifier.
#[tokio::test]
async fn stalled_draw_keeps_its_round() {
    use rafi_raffle::Event;

    let app = unreachable_drand_app().await;
    let raffle = setup_open(&app, &[("Prize", 3)]).await;
    for i in 0..4 {
        service::add_manual(&app, ACTOR, &raffle.id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    async fn draw_cmd(
        app: &AppState,
        raffle_id: &str,
        item: &str,
        id: &str,
    ) -> super::service::Result<()> {
        service::command(app, ACTOR, raffle_id, DrawRequest { item: item.into() }, id).await
    }
    let draw = |item: &str, id: &str| {
        let (item, id) = (item.to_owned(), id.to_owned());
        let (app, raffle_id) = (app.clone(), raffle.id.clone());
        async move { draw_cmd(&app, &raffle_id, &item, &id).await }
    };
    let stalls = |r: &Full| {
        r.drawn
            .as_ref()
            .map(|s| {
                s.events()
                    .iter()
                    .filter(|e| matches!(e, Event::Stalled { .. }))
                    .count()
            })
            .unwrap_or_default()
    };

    draw(items[0].as_str(), "d1").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 1).await;
    let pending = r
        .drawn
        .as_ref()
        .unwrap()
        .pending()
        .cloned()
        .expect("still pending");

    // Another item cannot start; this one retries the same round.
    assert!(draw(items[1].as_str(), "d2").await.is_err());
    draw(items[0].as_str(), "d3").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 2).await;
    let s = r.drawn.as_ref().unwrap();
    assert_eq!(s.pending(), Some(&pending), "same draw, same round");
    let announces = s
        .events()
        .iter()
        .filter(|e| matches!(e, Event::Announced { .. }))
        .count();
    assert_eq!(announces, 1);

    // The organizer abandons it: logged, and the item can be drawn again.
    service::command(
        &app,
        ACTOR,
        &raffle.id,
        Decision::Abandon {
            item: items[0].clone(),
        },
        "a1",
    )
    .await
    .unwrap();
    // A new draw of the item announces the same draw number on a current round.
    draw(items[0].as_str(), "d4").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 3).await;
    let next = r.drawn.as_ref().unwrap().pending().cloned().unwrap();
    assert_eq!(next.k, pending.k, "the abandoned draw's number is reused");
    assert!(
        next.round >= pending.round,
        "a new draw commits to a current round"
    );

    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let report = rafi_raffle::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_raffle::drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    let abandoned: Vec<_> = report.abandoned.iter().map(|c| c.round).collect();
    assert_eq!(abandoned, [pending.round]);
    assert!(rafi_raffle::verify_public(&data.manifest, &data.manifest_hash, &data.events).ok());
}

/// While a draw waits for its round, "not here" is refused; "claimed" is
/// accepted, and the engine applies it once that draw settles.
#[tokio::test]
async fn only_claiming_passes_a_waiting_draw() {
    let drand = rafi_engine::drand::DrandConfig {
        lead_secs: 3,
        ..mock_drand()
    };
    let app = app_with_engine(Config::for_tests(), drand, LiveUpdates::polling()).await;
    let raffle = setup_open(&app, &[("Prize", 2)]).await;
    for i in 0..3 {
        service::add_manual(&app, ACTOR, &raffle.id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let draw = |item: &str, id: &str| {
        let (item, id) = (item.to_owned(), id.to_owned());
        let (app, raffle_id) = (app.clone(), raffle.id.clone());
        async move {
            let request = DrawRequest { item: item.into() };
            service::command(&app, ACTOR, &raffle_id, request, &id).await
        }
    };
    let resolve = |resolution, id: &str| {
        let id = id.to_owned();
        let (app, raffle_id, item) = (app.clone(), raffle.id.clone(), items[0].clone());
        async move {
            let decision = Decision::Resolve { item, resolution };
            service::command(&app, ACTOR, &raffle_id, decision, &id).await
        }
    };

    draw(items[0].as_str(), "d1").await.unwrap();
    wait_long(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&items[0]).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    draw(items[1].as_str(), "d2").await.unwrap();
    wait_for(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.pending().as_ref().is_some_and(|p| p.item == items[1]))
    })
    .await;
    assert!(resolve(Resolution::NotHere, "nh").await.is_err());
    resolve(Resolution::Claimed, "cl").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(&items[0]).unwrap().status() == ItemStatus::Claimed)
    })
    .await;
    assert_eq!(
        r.drawn.as_ref().unwrap().item(&items[1]).unwrap().status(),
        ItemStatus::Drawn,
        "applied after the draw settled"
    );
}

/// The public page only trusts public drand chains: a mock raffle never passes there.
#[tokio::test]
async fn mock_raffles_are_not_publicly_verified() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    service::add_manual(&app, ACTOR, &raffle.id, "Person", None)
        .await
        .unwrap();
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let public = rafi_raffle::verify_public(&data.manifest, &data.manifest_hash, &data.events);
    assert!(!public.beacons_verified && !public.ok());
}

/// The hard limit: the join's `INSERT` refuses entrants beyond the cap, with
/// a clear message; someone already in is still told so.
#[tokio::test]
async fn joins_stop_at_the_entrant_cap() {
    let mut cfg = Config::for_tests();
    cfg.max_entrants = 2;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    for i in 0..2 {
        let (status, cookie, _) = join(
            &router,
            &raffle.join_code,
            &format!("P{i}"),
            &format!("p{i}@example.com"),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER, "join {i}");
        assert!(cookie.is_some());
    }
    let (status, cookie, page) = join(&router, &raffle.join_code, "Late", "late@example.com").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(cookie.is_none());
    assert!(page.contains("this raffle is full"), "{page}");
    assert!(page.contains("id=\"join\""), "the form stays");
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 2);

    let (status, _, page) = join(&router, &raffle.join_code, "Again", "p0@example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("already entered"), "{page}");
}

/// A proof-of-work is redeemed once: the second use fails the `pow_used`
/// primary key. Expired rows leave with the housekeeping run.
#[tokio::test]
async fn proof_of_work_replays_are_refused() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let proof = solved_pow(&router).await;
    let post = |name: &str, email: &str| {
        Request::post(format!("/j/{}", raffle.join_code))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form(&[
                ("name", name),
                ("email", email),
                ("rules", "on"),
                ("proof", &proof),
            ])))
            .unwrap()
    };
    let (status, _, _) = send(&router, post("Anna", "anna@example.com")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, _, page) = send(&router, post("Bot", "bot@example.com")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("human"), "{page}");
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 1);

    let payload = service::pow::decode(&proof).unwrap();
    let rows: Vec<(String, i64)> = app
        .db
        .query("SELECT challenge, expires_at FROM pow_used")
        .all()
        .await
        .unwrap();
    assert_eq!(
        rows,
        [(
            payload.challenge.parameters.nonce.clone(),
            payload.challenge.parameters.expires_at.unwrap()
        )]
    );
    assert!(!service::pow::redeem(&app, &proof).await.unwrap());

    // A forged solution never reaches the table.
    let mut forged = payload.clone();
    forged.challenge.signature = Some("00".repeat(32));
    assert!(
        !service::pow::redeem(&app, &service::pow::encode(&forged))
            .await
            .unwrap()
    );

    app.db
        .query("INSERT INTO pow_used (challenge, expires_at) VALUES ('old', 1)")
        .run()
        .await
        .unwrap();
    service::housekeeping::run(&app).await.unwrap();
    let (left,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM pow_used")
        .one()
        .await
        .unwrap();
    assert_eq!(left, 1, "only the unexpired challenge stays");
}

/// Only the server's own, unexpired, correctly solved challenges pass, each
/// once.
#[tokio::test]
async fn proof_of_work_forgeries_are_refused() {
    let app = app().await;
    let issue = |key: &str, expires_at: Option<i64>| {
        altcha::create_challenge(altcha::CreateChallengeOptions {
            algorithm: service::pow::ALGORITHM.into(),
            cost: 1,
            key_prefix: service::pow::KEY_PREFIX.into(),
            expires_at,
            hmac_signature_secret: Some(key.into()),
            ..Default::default()
        })
        .unwrap()
    };
    // Challenges expire by real time (the library's clock), not the app's.
    let now = service::Clock::system().now();
    let soon = Some(now + 600);
    let refused = async |payload: &altcha::Payload| {
        !service::pow::redeem(&app, &service::pow::encode(payload))
            .await
            .unwrap()
    };

    let expired = solve_pow_from(issue(super::config::TEST_POW_KEY, Some(now - 1)), 0);
    assert!(refused(&expired).await, "expired");
    let foreign = solve_pow_from(issue("another-key", soon), 0);
    assert!(refused(&foreign).await, "signed with another key");
    let eternal = solve_pow_from(issue(super::config::TEST_POW_KEY, None), 0);
    assert!(refused(&eternal).await, "no expiresAt");

    let good = solve_pow_from(issue(super::config::TEST_POW_KEY, soon), 0);
    let mut cheaper = good.clone();
    cheaper.challenge.parameters.cost = 0;
    assert!(refused(&cheaper).await, "a changed parameter");
    let mut counter = good.clone();
    counter.solution.counter += 1;
    assert!(refused(&counter).await, "a tampered counter");
    let mut key = good.clone();
    key.solution.derived_key = format!("00{}", "1".repeat(62));
    assert!(refused(&key).await, "the wrong derived key");

    // Accepted once; another valid counter for the same challenge is a replay.
    assert!(!refused(&good).await, "the genuine solution");
    let other = solve_pow_from(good.challenge.clone(), good.solution.counter + 1);
    assert_ne!(other.solution.counter, good.solution.counter);
    assert!(
        refused(&other).await,
        "a second solution of the same challenge"
    );

    assert!(
        !service::pow::redeem(&app, "not base64!").await.unwrap(),
        "junk"
    );
    let long = "A".repeat(service::pow::MAX_PAYLOAD_LEN + 4);
    assert!(
        !service::pow::redeem(&app, &long).await.unwrap(),
        "too long"
    );
}

/// A raffle with three entrants, frozen and finished without draws.
async fn finished_raffle(app: &AppState) -> RaffleRow {
    let raffle = setup_open(app, &[("Prize", 1)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        service::add_manual(app, ACTOR, &raffle.id, name, None)
            .await
            .unwrap();
    }
    service::start_draw(app, ACTOR, &raffle.id).await.unwrap();
    service::command(app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Finished);
    r
}

#[tokio::test]
async fn ending_live_updates() {
    let (live, live_updates) = recording();
    let app = app_with_live(Config::for_tests(), live_updates).await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    assert!(
        !live.notified().is_empty(),
        "lifecycle changes nudge the channel"
    );
    let err = service::end_live(&app, ACTOR, &raffle.id)
        .await
        .unwrap_err();
    assert!(matches!(err, service::Error::Refused(_)), "{err}");
    assert!(live.ended().is_empty());

    let raffle = finished_raffle(&app).await;
    assert!(raffle.live_ended_at.is_none());
    let view = super::view::build(&app, &raffle.id).await.unwrap().unwrap();
    assert_eq!(view.follow, Follow::Partyline { base_url: None });
    assert!(!view.live_ended);

    service::end_live(&app, ACTOR, &raffle.id).await.unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    let ended = r.live_ended_at.expect("live_ended_at is set");
    assert_eq!(live.ended(), [RaffleId::new(&raffle.id)]);
    let view = super::view::build(&app, &raffle.id).await.unwrap().unwrap();
    assert_eq!(view.follow, Follow::poll(), "ended: plain polling");
    assert!(view.live_ended);

    // Again: the channel is destroyed again, the time stays.
    service::end_live(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(live.ended().len(), 2);
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.live_ended_at, Some(ended));
    // Other organizers can't.
    assert!(matches!(
        service::end_live(&app, service::Actor::solo("someone_else"), &raffle.id).await,
        Err(service::Error::NotFound)
    ));
}

/// Deleting and purging a raffle destroy its channel; admin edits that phones
/// see nudge it, joins don't.
#[tokio::test]
async fn writes_nudge_and_destroy_the_channel() {
    let (live, live_updates) = recording();
    let app = app_with_live(Config::for_tests(), live_updates).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let nudges = || live.notified().len();

    let before = nudges();
    join(&router, &raffle.join_code, "Anna", "anna@example.com").await;
    service::add_manual(&app, ACTOR, &raffle.id, "Bence", None)
        .await
        .unwrap();
    assert_eq!(nudges(), before, "joins don't nudge");

    // Entrants who joined in the same second are listed by ID.
    let anna = entrants::list(&app.db, &raffle.id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.display_name == "Anna")
        .unwrap()
        .id;
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &anna,
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(nudges(), before + 1, "hiding a name nudges");
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(nudges(), before + 2, "the freeze nudges");

    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    assert_eq!(live.ended(), [RaffleId::new(&raffle.id)]);
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Purged);
    assert!(r.live_ended_at.is_some(), "a purge ends live updates");

    let other = create(&app, OWNER, "Other").await;
    service::delete(&app, ACTOR, &other).await.unwrap();
    assert_eq!(
        live.ended(),
        [RaffleId::new(&raffle.id), RaffleId::new(other)]
    );
}

/// The housekeeping run ends the live channel of raffles finished over a day
/// ago, once.
#[tokio::test]
async fn housekeeping_ends_stale_live_channels() {
    let (live, live_updates) = recording();
    let app = app_with_live(Config::for_tests(), live_updates).await;
    let old = finished_raffle(&app).await;
    // A day and a minute later, another finishes.
    app.clock
        .advance(service::housekeeping::CHANNEL_GRACE_SECS + 60);
    let recent = finished_raffle(&app).await;

    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.channels_ended, 1);
    assert_eq!(live.ended(), [RaffleId::new(&old.id)]);
    let r = raffles::get(&app.db, &old.id).await.unwrap().unwrap();
    assert!(r.live_ended_at.is_some());
    assert!(
        raffles::get(&app.db, &recent.id)
            .await
            .unwrap()
            .unwrap()
            .live_ended_at
            .is_none()
    );
    assert!(
        entrants::list(&app.db, &old.id)
            .await
            .unwrap()
            .iter()
            .all(|e| !e.erased),
        "housekeeping erases nobody"
    );
    assert_eq!(
        service::housekeeping::run(&app)
            .await
            .unwrap()
            .channels_ended,
        0,
        "once"
    );
}

/// Purging logs every erasure once, by ID, without personal data.
#[tokio::test]
async fn erasures_are_logged_without_personal_data() {
    let app = app().await;
    let raffle = finished_raffle(&app).await;
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert!(
        entrants
            .iter()
            .all(|e| e.erased && e.display_name.starts_with('#'))
    );
    let log: Vec<serde_json::Value> = app.db.query("SELECT * FROM erasures").all().await.unwrap();
    assert_eq!(log.len(), entrants.len());
    for row in &log {
        let mut keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            ["at", "entrant_id", "id", "raffle_id"],
            "no personal data"
        );
        assert_eq!(row["raffle_id"], raffle.id.as_str());
    }
    let mut logged: Vec<&str> = log
        .iter()
        .filter_map(|r| r["entrant_id"].as_str())
        .collect();
    logged.sort_unstable();
    let mut ids: Vec<&str> = entrants.iter().map(|e| e.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(logged, ids, "every entrant, once");

    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let again: Vec<serde_json::Value> = app.db.query("SELECT * FROM erasures").all().await.unwrap();
    assert_eq!(again.len(), log.len(), "purging again has nothing to do");
}

/// Leaving the name blank gets a generated one, which the page then shows;
/// the form offers generated names to pick from.
#[tokio::test]
async fn a_blank_name_gets_a_generated_one() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains("id=\"name-dice\""), "the 🎲 button");
    let name_input = page
        .split("id=\"name\"")
        .nth(1)
        .and_then(|rest| rest.split('>').next())
        .unwrap();
    assert!(!name_input.contains("required"), "optional: {name_input}");
    let config: rafi_live_client::PageConfig = serde_json::from_str(
        page.split("id=\"rafi-config\">")
            .nth(1)
            .and_then(|rest| rest.split("</script>").next())
            .unwrap(),
    )
    .unwrap();
    assert!(!config.name_ideas.is_empty());
    for idea in &config.name_ideas {
        assert_eq!(
            rafi_lifecycle::entrant::display_name(idea).as_ref(),
            Ok(idea)
        );
    }

    let (status, cookie, _) = join(&router, &raffle.join_code, "   ", "blank@example.com").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let entrant = entrants::list(&app.db, &raffle.id).await.unwrap().remove(0);
    let generated: std::collections::HashSet<String> = (0
        ..rafi_lifecycle::entrant::GENERATED_NAMES)
        .map(rafi_lifecycle::entrant::generated_name)
        .collect();
    assert!(
        generated.contains(&entrant.display_name),
        "{}",
        entrant.display_name
    );
    let (_, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .header(header::COOKIE, cookie.unwrap())
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(page.contains(&entrant.display_name), "shown on the ticket");
    assert!(
        page.contains(&format!("/v/{}#me={}", raffle.slug, entrant.id)),
        "the verification link finds them"
    );

    // A name that is too long is still refused.
    let (status, _, page) = join(
        &router,
        &raffle.join_code,
        &"x".repeat(rafi_lifecycle::entrant::MAX_NAME_CHARS + 1),
        "long@example.com",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("leave it blank"), "{page}");
}

/// From the freeze until the raffle finishes, nobody leaves the list; once it
/// has, one entrant's personal data can be erased, and the published names
/// drop them.
#[tokio::test]
async fn entrants_are_erased_only_once_the_raffle_is_over() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let mut ids = Vec::new();
    for name in ["Anna", "Bence", "Csilla"] {
        ids.push(
            service::add_manual(&app, ACTOR, &raffle.id, name, None)
                .await
                .unwrap(),
        );
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let refused = service::remove_entrant(&app, ACTOR, &raffle.id, &ids[0]).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::NotRemovable))
        ),
        "{refused:?}"
    );
    // Hiding still works.
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &ids[1],
        rafi_api::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .iter()
            .all(|e| !e.erased)
    );

    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    service::remove_entrant(&app, ACTOR, &raffle.id, &ids[0])
        .await
        .unwrap();
    let anna = entrants::get(&app.db, &raffle.id, &ids[0])
        .await
        .unwrap()
        .unwrap();
    assert!(anna.erased && anna.claim_code.is_none());

    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let names: std::collections::HashMap<&str, &str> = data
        .names
        .iter()
        .map(|(id, n)| (id.as_str(), n.as_str()))
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(!names.contains_key(ids[0].as_str()), "erased: no name");
    assert_eq!(names[ids[1].as_str()], rafi_live_client::HIDDEN_NAME);
    assert_eq!(names[ids[2].as_str()], "Csilla");
    let manifest: rafi_raffle::Manifest = serde_json::from_str(&data.manifest).unwrap();
    assert_eq!(
        manifest.entrants.len(),
        3,
        "the erased ID stays in the list"
    );
    assert!(!data.test_mode);
}

/// Switching test mode off removes the fake entrants; a raffle out of test
/// mode doesn't freeze with fakes; fakes get generated names; the
/// verification data says a rehearsal is one.
#[tokio::test]
async fn fakes_stay_in_test_mode() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    let set_test_mode = |on: bool| {
        let mut settings = service::settings_of(&raffle);
        settings.test_mode = on;
        service::update_settings(&app, ACTOR, &raffle.id, settings)
    };
    set_test_mode(true).await.unwrap();
    service::add_fakes(&app, ACTOR, &raffle.id, 5)
        .await
        .unwrap();
    service::add_manual(&app, ACTOR, &raffle.id, "Real Person", None)
        .await
        .unwrap();
    let listed = entrants::list(&app.db, &raffle.id).await.unwrap();
    let fakes: Vec<_> = listed
        .iter()
        .filter(|e| e.source == rafi_lifecycle::entrant::Source::Fake)
        .collect();
    assert_eq!(fakes.len(), 5);
    let generated: std::collections::HashSet<String> = (0
        ..rafi_lifecycle::entrant::GENERATED_NAMES)
        .map(rafi_lifecycle::entrant::generated_name)
        .collect();
    assert!(
        fakes
            .iter()
            .all(|e| generated.contains(&e.display_name) && e.consent_version.is_none()),
        "{fakes:?}"
    );

    set_test_mode(false).await.unwrap();
    let left = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(left.len(), 1, "the fakes went with test mode");
    assert_eq!(left[0].display_name, "Real Person");
    let audit: Vec<(String, String)> = app
        .db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(&raffle.id)
        .all()
        .await
        .unwrap();
    assert_eq!(
        &audit[audit.len() - 2..],
        [
            ("remove_fakes".into(), "5".into()),
            ("settings".into(), "test_mode".into())
        ],
        "the removal, and the setting that changed, by name: {audit:?}"
    );
    assert!(
        service::add_fakes(&app, ACTOR, &raffle.id, 1)
            .await
            .is_err(),
        "no fakes outside test mode"
    );

    // Fakes left behind all the same (say, by a restored backup): no freeze.
    app.db
        .query(
            "INSERT INTO entrants (id, raffle_id, display_name, source, claim_code, created_at)
             VALUES ('ffffffffffffffffffffffffffffffff', ?, 'Stray', 'fake', 'S1', 0)",
        )
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    let refused = service::start_draw(&app, ACTOR, &raffle.id).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::Start(
                rafi_lifecycle::start::Refusal::FakeEntrants
            )))
        ),
        "{refused:?}"
    );
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Open
    );

    // A rehearsal freezes with its fakes, and says so.
    set_test_mode(true).await.unwrap();
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(data.test_mode);
    assert!(
        matches!(
            set_test_mode(false).await,
            Err(service::Error::Refused(service::Refusal::TestModeLocked))
        ),
        "test mode is fixed once frozen"
    );
}

// ---------------------------------------------------------------------------
// Privacy and retention
// ---------------------------------------------------------------------------

const DAY: i64 = 86_400;

/// Keeps raffle `id`'s personal data `days` past its event (today, as
/// [`create`] dates it), before anyone joins, so the lock isn't engaged.
/// Returns the row. Time then passes on `app.clock`.
async fn retain(app: &AppState, id: &str, days: i64) -> RaffleRow {
    let mut settings = service::settings_of(&raffles::get(&app.db, id).await.unwrap().unwrap());
    settings.retention_days = days;
    service::update_settings(app, ACTOR, id, settings)
        .await
        .unwrap();
    raffles::get(&app.db, id).await.unwrap().unwrap()
}

/// Draws `item` and claims it; returns the winner.
async fn draw_and_claim(app: &AppState, raffle_id: &str, item: &rafi_raffle::ItemId) -> String {
    service::command(
        app,
        ACTOR,
        raffle_id,
        DrawRequest { item: item.clone() },
        &format!("draw-{item}"),
    )
    .await
    .unwrap();
    let r = wait_for(app, raffle_id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    let winner = r
        .drawn
        .as_ref()
        .unwrap()
        .item(item)
        .unwrap()
        .winner()
        .cloned()
        .unwrap();
    service::command(
        app,
        ACTOR,
        raffle_id,
        Decision::Resolve {
            item: item.clone(),
            resolution: Resolution::Claimed,
        },
        &format!("claim-{item}"),
    )
    .await
    .unwrap();
    wait_for(app, raffle_id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Claimed)
    })
    .await;
    winner.to_string()
}

/// The audit rows of a raffle, `(action, detail)`, oldest first.
async fn audit_of(app: &AppState, raffle_id: &str) -> Vec<(String, String)> {
    app.db
        .query("SELECT action, detail FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(raffle_id)
        .all()
        .await
        .unwrap()
}

/// The join page, the rules and both privacy notices tell entrants what
/// happens to their data, who is responsible, and how old they must be.
#[tokio::test]
async fn the_join_page_and_the_notices_say_when_data_is_erased() {
    use rafi_lifecycle::dates::Date;
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let mut settings = service::settings_of(&raffle);
    settings.event_date = "2026-10-28".into();
    settings.retention_days = 7;
    settings.privacy_email = "dpo@example.com".into();
    settings.privacy_text = "We never sell anything.\nAsk us anything.".into();
    settings.privacy_url = "https://example.com/privacy".into();
    service::update_settings(&app, ACTOR, &raffle.id, settings)
        .await
        .unwrap();

    let (status, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("I accept the "), "{page}");
    assert!(
        page.contains(">raffle rules</a>"),
        "the checkbox is about the rules"
    );
    assert!(
        !page.contains("accept the raffle rules and the privacy notice"),
        "the notice is read, not accepted"
    );
    assert!(
        page.contains("Your name and email are erased on 4 November 2026; a winner&#39;s once their prize is delivered, at the latest on 2 February 2027."),
        "{page}"
    );
    assert!(page.contains("you confirm you are 16 or older"), "{page}");
    assert!(!page.contains("marketing") && !page.contains("future meetups"));
    assert!(page.contains(&format!("/privacy/{}", raffle.slug)));

    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("Cloud Native Budapest is the data controller"),
        "{notice}"
    );
    assert!(
        notice.contains("Privacy requests: dpo@example.com"),
        "the privacy contact wins"
    );
    assert!(notice.contains("The event is on 28 October 2026."));
    assert!(notice.contains("erased on 4 November 2026"));
    assert!(notice.contains("erased on 2 February 2027 at the latest"));
    assert!(
        notice.contains("Rafi Test Operator (privacy@rafi.test) runs this service"),
        "{notice}"
    );
    assert!(notice.contains("version test-1"));
    assert!(
        notice.contains("href=\"/privacy\""),
        "links the installation's notice"
    );
    assert!(!notice.contains("Clerk"), "no hard-coded subprocessors");
    assert!(
        !notice.contains("Art. 6(1)(a)"),
        "no consent-based processing"
    );
    let own = notice
        .find("From the organizer")
        .expect("the organizer's own section");
    let generated = notice.find("Who processes it").unwrap();
    assert!(own > generated, "after the generated sections");
    assert!(notice.contains("We never sell anything."));
    assert!(notice.contains("href=\"https://example.com/privacy\""));

    let (status, _, rules) = send(
        &router,
        Request::get(format!("/rules/{}", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(rules.contains("Entrants must be 16 or older:"), "{rules}");

    // The installation's own documents, with their versions.
    for (path, text) in [
        ("/privacy", "The test installation"),
        ("/terms", "<strong>processing agreement</strong>"),
    ] {
        let (status, _, body) =
            send(&router, Request::get(path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains(text), "{path}: {body}");
        assert!(body.contains("version test-1"), "{path}");
    }

    // The admin sees the same dates.
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.retention.erase_date, "4 November 2026");
    assert_eq!(admin.retention.winners_erase_date, "2 February 2027");
    assert_eq!(
        admin.retention.erase_at,
        Date::parse("2026-10-28").unwrap().ends_utc() + 7 * DAY
    );
    assert_eq!(admin.retention.max_days, app.policy.retention.max_days);
    assert_eq!(
        admin.retention.winner_hold_days,
        app.policy.retention.winner_hold_days
    );
}

/// The clocks on frozen raffles: non-winners go on the erasure date,
/// delivered winners with them, other winners after the hold; a finished
/// raffle is then purged, a drawing one is erased where it stands and
/// finished by the engine. (Never-frozen raffles are
/// [`never_frozen_raffles_expire`].)
#[tokio::test]
async fn housekeeping_erases_on_the_dates() {
    let (recording, live) = recording();
    let app = app_with_live(Config::for_tests(), live).await;
    let hold = app.policy.retention.winner_hold_secs();

    // A finished raffle: two prizes, one winner delivered, one not, one
    // drawn but not here. Today's, kept seven days.
    let finished = setup_open(&app, &[("Book", 1), ("Mug", 1)]).await;
    let router = routes(app.clone());
    for (name, email) in [
        ("Anna", "anna@x.org"),
        ("Bence", "bence@x.org"),
        ("Csilla", "csilla@x.org"),
        ("Dani", "dani@x.org"),
    ] {
        let (status, _, _) = join(&router, &finished.join_code, name, email).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
    }
    service::start_draw(&app, ACTOR, &finished.id)
        .await
        .unwrap();
    let r = wait_for(&app, &finished.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let delivered = draw_and_claim(&app, &finished.id, &items[0]).await;
    let held = draw_and_claim(&app, &finished.id, &items[1]).await;
    service::patch_entrant(
        &app,
        ACTOR,
        &finished.id,
        &delivered,
        rafi_api::EntrantPatch {
            delivered: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::command(&app, ACTOR, &finished.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &finished.id).await.unwrap();
    assert_eq!(admin.retention.held_winners, 1);
    assert!(admin.actions.deliver_all);
    let held_entrant = admin.entrants.iter().find(|e| e.id == held).unwrap();
    assert_eq!(held_entrant.held_until, Some(finished.erase_at + hold));

    // A drawing raffle, never finished: one claimed winner.
    let drawing = setup_open(&app, &[("Pen", 1)]).await;
    for name in ["Erik", "Fanni"] {
        service::add_manual(&app, ACTOR, &drawing.id, name, None)
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &drawing.id).await.unwrap();
    let r = wait_for(&app, &drawing.id, |r| r.raffle.opened()).await;
    let pen = r.drawn.as_ref().unwrap().items()[0].id.clone();
    let drawing_winner = draw_and_claim(&app, &drawing.id, &pen).await;

    // Ten days on: due, not at the ceiling.
    app.clock.advance(10 * DAY);
    let now = app.clock.now();
    assert!(
        finished.erase_at < now && finished.erase_at + hold > now,
        "due, not at the ceiling"
    );
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(
        (report.expired, report.erased, report.purged),
        (0, 4, 0),
        "2 non-winners + the delivered winner, and the drawing raffle's non-winner, erased"
    );

    // Finished: everyone but the held winner is erased, the raffle stays finished.
    let entrants = entrants::list(&app.db, &finished.id).await.unwrap();
    for e in &entrants {
        assert_eq!(e.erased, e.id != held, "{}: {}", e.display_name, e.id);
        if e.erased {
            assert!(e.email_raw.is_none() && e.claim_code.is_none() && e.token_hash.is_none());
        }
    }
    let r = raffles::get(&app.db, &finished.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Finished);
    assert!(r.purged_at.is_none());
    let audit = audit_of(&app, &finished.id).await;
    assert_eq!(
        audit.last().unwrap(),
        &("auto_erase".to_owned(), "3".to_owned())
    );
    let data = service::verify_data(&app, &finished.slug).await.unwrap();
    let report_v = rafi_raffle::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_raffle::drand::resolve_chain,
    );
    assert!(report_v.ok(), "the draw still verifies");
    assert_eq!(data.names.len(), 1, "only the held winner is named");
    assert!(
        recording.notified().contains(&RaffleId::new(&finished.id)),
        "screens were nudged"
    );

    // Drawing: erased where it stands.
    let entrants = entrants::list(&app.db, &drawing.id).await.unwrap();
    for e in &entrants {
        assert_eq!(e.erased, e.id != drawing_winner, "{}", e.display_name);
    }
    assert_eq!(
        raffles::get(&app.db, &drawing.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Drawing
    );

    // A second run changes nothing.
    let again = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((again.erased, again.purged), (0, 0));
    let (logged,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = ?")
        .bind(&finished.id)
        .one()
        .await
        .unwrap();
    assert_eq!(logged, 3, "each erasure logged once");
    assert!(
        !service::housekeeping::stale(&app).await.unwrap(),
        "a run just succeeded"
    );

    // The ceiling: the held winner goes, the finished raffle is purged, the
    // drawing one is finished by the engine and purged on the run after.
    app.clock.set(finished.erase_at + hold + 1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(
        (report.erased, report.purged),
        (2, 1),
        "both held winners, the finished raffle purged: {report:?}"
    );
    assert!(
        entrants::list(&app.db, &finished.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );
    let r = raffles::get(&app.db, &finished.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Purged);
    assert!(r.purged_at.is_some());
    assert_eq!(
        audit_of(&app, &finished.id).await.last().unwrap(),
        &("auto_purge".to_owned(), String::new())
    );
    assert!(
        recording.ended().contains(&RaffleId::new(&finished.id)),
        "its channel ended"
    );
    assert!(
        entrants::list(&app.db, &drawing.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );
    let r = wait_for(&app, &drawing.id, |r| {
        r.raffle.state == RaffleState::Finished
    })
    .await;
    assert!(
        r.raffle.finished_at.is_some(),
        "the engine finished it, best effort"
    );
    app.clock.advance(1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.purged, 1, "purged on the run after it finished");
    assert_eq!(
        raffles::get(&app.db, &drawing.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    let data = service::verify_data(&app, &finished.slug).await.unwrap();
    assert!(data.names.is_empty());
    assert!(
        rafi_raffle::verify(
            &data.manifest,
            &data.manifest_hash,
            &data.events,
            rafi_raffle::drand::resolve_chain
        )
        .ok()
    );
}

/// Erasing one entrant on request: refused while the list is frozen before
/// the erasure date, allowed once the date has passed; the admin says which.
#[tokio::test]
async fn erasure_on_request_follows_the_date() {
    use rafi_lifecycle::entrant::Removal;
    let app = app().await;
    // Anna and Bence in a raffle frozen today; returns Anna's ID.
    let frozen = async |r: &RaffleRow| {
        let id = service::add_manual(&app, ACTOR, &r.id, "Anna", Some("anna@x.org".into()))
            .await
            .unwrap();
        service::add_manual(&app, ACTOR, &r.id, "Bence", None)
            .await
            .unwrap();
        service::start_draw(&app, ACTOR, &r.id).await.unwrap();
        wait_for(&app, &r.id, |r| r.raffle.opened()).await;
        id
    };

    // Ten days ago's raffle, and today's.
    let past = setup_open(&app, &[("Book", 1)]).await;
    let past_anna = frozen(&past).await;
    app.clock.advance(10 * DAY);
    let before = setup_open(&app, &[("Book", 1)]).await;
    let before_anna = frozen(&before).await;
    let now = app.clock.now();
    assert!(before.erase_at > now && past.erase_at <= now);
    let ids = [before_anna, past_anna];

    let refused = service::remove_entrant(&app, ACTOR, &before.id, &ids[0]).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::NotRemovable))
        ),
        "{refused:?}"
    );
    assert_eq!(
        service::admin(&app, OWNER, &before.id)
            .await
            .unwrap()
            .actions
            .removal,
        None
    );
    assert!(
        !entrants::get(&app.db, &before.id, &ids[0])
            .await
            .unwrap()
            .unwrap()
            .erased
    );

    assert_eq!(
        service::admin(&app, OWNER, &past.id)
            .await
            .unwrap()
            .actions
            .removal,
        Some(Removal::Erase)
    );
    service::remove_entrant(&app, ACTOR, &past.id, &ids[1])
        .await
        .unwrap();
    let anna = entrants::get(&app.db, &past.id, &ids[1])
        .await
        .unwrap()
        .unwrap();
    assert!(anna.erased && anna.email_raw.is_none());
    assert_eq!(
        audit_of(&app, &past.id).await.last().unwrap(),
        &("erase_entrant".to_owned(), ids[1].clone())
    );
    // The raffle is still being drawn; the other entrant is untouched until
    // housekeeping or a request.
    assert_eq!(
        raffles::get(&app.db, &past.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Frozen
    );
    assert_eq!(
        entrants::list(&app.db, &past.id)
            .await
            .unwrap()
            .iter()
            .filter(|e| !e.erased)
            .count(),
        1
    );
}

/// "All prizes delivered" puts every held winner on the non-winner clock.
#[tokio::test]
async fn all_prizes_delivered() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1), ("Mug", 1)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        service::add_manual(&app, ACTOR, &raffle.id, name, None)
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let w1 = draw_and_claim(&app, &raffle.id, &items[0]).await;
    let w2 = draw_and_claim(&app, &raffle.id, &items[1]).await;

    let refused = service::deliver_all(&app, ACTOR, &raffle.id).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::NotOver(
                service::Unfinished::DeliverAll
            )))
        ),
        "not before the finish: {refused:?}"
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "All prizes can be marked delivered at once once the raffle has finished. One winner's prize can be marked delivered any time after they claimed it."
    );
    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.retention.held_winners, 2);
    assert!(admin.actions.deliver_all);

    assert_eq!(
        service::deliver_all(&app, ACTOR, &raffle.id).await.unwrap(),
        2
    );
    for w in [&w1, &w2] {
        assert!(
            entrants::get(&app.db, &raffle.id, w)
                .await
                .unwrap()
                .unwrap()
                .delivered_at
                .is_some()
        );
    }
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.retention.held_winners, 0);
    assert!(!admin.actions.deliver_all);
    assert!(admin.entrants.iter().all(|e| e.held_until.is_none()));
    assert_eq!(
        audit_of(&app, &raffle.id).await.last().unwrap(),
        &("deliver_all".to_owned(), "2".to_owned())
    );
    assert_eq!(
        service::deliver_all(&app, ACTOR, &raffle.id).await.unwrap(),
        0,
        "idempotent"
    );
}

/// Registration opens only once the notice can name the organizer and a
/// way to reach them.
#[tokio::test]
async fn registration_needs_an_organizer() {
    let app = app().await;
    let id = service::create(&app, ACTOR, "Nameless", &service::EventDate::today(&app))
        .await
        .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &id).await.unwrap();
    assert_eq!(
        admin.missing_for_open,
        ["the organizer's name", "a contact or privacy email"]
    );
    let refused = service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open).await;
    assert!(
        matches!(
            &refused,
            Err(service::Error::Refused(service::Refusal::OrganizerMissing(fields)))
                if fields == &["the organizer's name", "a contact or privacy email"]
        ),
        "{refused:?}"
    );
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Ready
    );

    // A privacy contact alone is a way to reach them.
    let mut settings = admin.settings;
    settings.organizer_name = "Org".into();
    settings.privacy_email = "dpo@example.com".into();
    service::update_settings(&app, ACTOR, &id, settings)
        .await
        .unwrap();
    assert!(
        service::admin(&app, OWNER, &id)
            .await
            .unwrap()
            .missing_for_open
            .is_empty()
    );
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Open)
        .await
        .unwrap();
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state,
        RaffleState::Open
    );
}

/// The export has the winners with their details, the delivered flag and
/// formula-safe cells, and every export is audited: who, which raffle, how
/// many rows.
#[tokio::test]
async fn exports_have_the_winners_and_are_audited() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1), ("Mug", 1)]).await;
    // A name that a spreadsheet would run as a formula.
    let formula = service::add_manual(
        &app,
        ACTOR,
        &raffle.id,
        "=SUM(A1)",
        Some("sum@x.org".into()),
    )
    .await
    .unwrap();
    service::add_manual(&app, ACTOR, &raffle.id, "Bence", Some("bence@x.org".into()))
        .await
        .unwrap();
    service::add_manual(&app, ACTOR, &raffle.id, "Csilla", None)
        .await
        .unwrap();
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    let items: Vec<_> = r
        .drawn
        .as_ref()
        .unwrap()
        .items()
        .iter()
        .map(|u| u.id.clone())
        .collect();
    let w1 = draw_and_claim(&app, &raffle.id, &items[0]).await;
    let w2 = draw_and_claim(&app, &raffle.id, &items[1]).await;
    service::patch_entrant(
        &app,
        ACTOR,
        &raffle.id,
        &w1,
        rafi_api::EntrantPatch {
            delivered: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    service::command(&app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();

    let csv = service::export_csv(&app, ACTOR, &raffle.id).await.unwrap();
    assert!(csv.starts_with('\u{FEFF}'), "a byte order mark, for Excel");
    let lines: Vec<&str> = csv.trim_start_matches('\u{FEFF}').lines().collect();
    assert_eq!(
        lines[0],
        "entrant_id,display_name,email,source,claim_code,outcome,prize,delivered"
    );
    assert_eq!(lines.len(), 3, "the two winners");
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    for (winner, delivered) in [(&w1, "yes"), (&w2, "no")] {
        let e = entrants.iter().find(|e| &e.id == winner).unwrap();
        let line = lines
            .iter()
            .find(|l| l.starts_with(&format!("\"{winner}\"")))
            .unwrap();
        let email = e.email_raw.clone().unwrap_or_default();
        let claim = e.claim_code.clone().unwrap();
        let name = if e.id == formula {
            "'=SUM(A1)"
        } else {
            &e.display_name
        };
        assert!(
            line.contains(&format!(
                "\"{name}\",\"{email}\",\"manual\",\"{claim}\",\"claimed\""
            )),
            "{line}"
        );
        assert!(line.ends_with(&format!("\"{delivered}\"")), "{line}");
    }
    assert_eq!(
        audit_of(&app, &raffle.id).await.last().unwrap(),
        &("export_csv".to_owned(), "2".to_owned())
    );
    assert!(
        service::export_csv(&app, service::Actor::solo("someone-else"), &raffle.id)
            .await
            .is_err(),
        "another organizer's raffle doesn't exist for them"
    );
}

/// An organizer removes their data: drafts go, finished raffles lose their
/// personal data and the organizer, defaults go, the terms' record and the
/// verifiable draw stay. Not while a raffle is being drawn.
#[tokio::test]
async fn an_organizer_can_remove_their_data() {
    let app = app().await;
    let router = routes(app.clone());
    service::save_defaults(
        &app,
        ACTOR,
        rafi_api::RaffleDefaults {
            organizer_name: "Org".into(),
            contact_email: "a@b.c".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let finished = finished_raffle(&app).await;
    let draft = create(&app, OWNER, "Draft").await;
    let drawing = setup_open(&app, &[("Book", 1)]).await;
    service::add_manual(&app, ACTOR, &drawing.id, "Anna", None)
        .await
        .unwrap();
    service::start_draw(&app, ACTOR, &drawing.id).await.unwrap();
    wait_for(&app, &drawing.id, |r| r.raffle.opened()).await;

    let refused = service::remove_owner_data(&app, ACTOR).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::EventLive))
        ),
        "{refused:?}"
    );
    assert!(
        raffles::get(&app.db, &draft).await.unwrap().is_some(),
        "nothing happened"
    );

    service::command(&app, ACTOR, &drawing.id, Decision::Finish, "fin")
        .await
        .unwrap();
    service::remove_owner_data(&app, ACTOR).await.unwrap();

    assert!(
        raffles::get(&app.db, &draft).await.unwrap().is_none(),
        "the draft is gone"
    );
    for r in [&finished, &drawing] {
        let row = raffles::get(&app.db, &r.id).await.unwrap().unwrap();
        assert_eq!(row.state, RaffleState::Purged);
        assert!(
            row.organizer_name.is_empty()
                && row.contact_email.is_empty()
                && row.privacy_email.is_empty()
        );
        assert!(
            entrants::list(&app.db, &r.id)
                .await
                .unwrap()
                .iter()
                .all(|e| e.erased)
        );
        let data = service::verify_data(&app, &r.slug).await.unwrap();
        assert!(
            rafi_raffle::verify(
                &data.manifest,
                &data.manifest_hash,
                &data.events,
                rafi_raffle::drand::resolve_chain
            )
            .ok()
        );
        assert_eq!(
            audit_of(&app, &r.id).await.last().unwrap(),
            &("purge".to_owned(), "owner left".to_owned())
        );
        assert!(row.organizer_left_at.is_some() && row.purged_at.is_some());
    }
    assert_eq!(
        audit_of(&app, &draft).await.last().unwrap(),
        &("delete".to_owned(), "owner left".to_owned())
    );
    let (logged,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = ?")
        .bind(&finished.id)
        .one()
        .await
        .unwrap();
    assert_eq!(logged, 3, "each erasure logged");
    assert_eq!(
        service::get_defaults(&app, OWNER).await.unwrap(),
        Default::default()
    );
    assert!(
        service::list(&app, OWNER)
            .await
            .unwrap()
            .iter()
            .all(|r| r.state == RaffleState::Purged)
    );

    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", finished.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("who has since left this service"),
        "{notice}"
    );
    assert!(!notice.contains("hello@example.com"));
}

/// After a restore, the erasure log, the purges and the organizers who left
/// are re-applied, and whatever is due is erased again.
#[tokio::test]
async fn erasures_are_reapplied_after_a_restore() {
    let app = app().await;
    let raffle = finished_raffle(&app).await;
    let ids: Vec<String> = entrants::list(&app.db, &raffle.id)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    service::remove_entrant(&app, ACTOR, &raffle.id, &ids[0])
        .await
        .unwrap();
    service::purge(&app, ACTOR, &raffle.id).await.unwrap();
    let left = finished_raffle(&app).await;
    service::remove_owner_data(&app, ACTOR).await.unwrap();

    // "Restore": the personal data is back, the first raffle finished again,
    // the organizer named again on the second.
    app.db
        .query("UPDATE entrants SET erased = 0, display_name = 'Back', email_raw = 'back@x.org', claim_code = 'K9' WHERE raffle_id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    app.db
        .query("UPDATE raffles SET state = 'finished' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    app.db
        .query(
            "UPDATE raffles SET organizer_name = 'Back', contact_email = 'back@x.org' WHERE id = ?",
        )
        .bind(&left.id)
        .run()
        .await
        .unwrap();

    let done = service::reapply_erasures(&app).await.unwrap();
    assert_eq!((done.entrants, done.raffles, done.organizers), (3, 1, 1));
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert!(
        entrants
            .iter()
            .all(|e| e.erased && e.email_raw.is_none() && e.claim_code.is_none())
    );
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    let left = raffles::get(&app.db, &left.id).await.unwrap().unwrap();
    assert!(left.organizer_name.is_empty() && left.contact_email.is_empty());
    let again = service::reapply_erasures(&app).await.unwrap();
    assert_eq!(
        (again.entrants, again.raffles, again.organizers),
        (0, 0, 0),
        "idempotent"
    );
}

/// `whoami` warns when housekeeping hasn't succeeded for two hours, counted
/// from the first raffle: a fresh installation isn't stale.
#[tokio::test]
async fn a_stopped_run_is_reported() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let me = async |router: &Router| -> rafi_api::Me {
        let (status, _, body) = send(router, admin_call("whoami", "")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        serde_json::from_str(&body).unwrap()
    };
    assert!(!me(&router).await.retention_stale, "nothing to erase yet");
    create(&app, "dev", "First").await;
    app.clock.advance(service::housekeeping::STALE_AFTER_SECS);
    assert!(
        !me(&router).await.retention_stale,
        "a raffle no older than two hours"
    );
    app.clock.advance(1);
    assert!(
        me(&router).await.retention_stale,
        "two hours with something to erase and no run"
    );
    service::housekeeping::run(&app).await.unwrap();
    assert!(!me(&router).await.retention_stale);
    app.clock
        .advance(service::housekeeping::STALE_AFTER_SECS + 1);
    assert!(me(&router).await.retention_stale);
}

/// A raffle dated in the past, before anyone joined, is warned about: it
/// expires on its erasure date, or within the hour once that has passed.
#[tokio::test]
async fn a_past_event_date_is_warned_about() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    app.clock.advance(2 * DAY);
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(admin.retention.past_event && !admin.retention.past_erase_date);
    assert!(admin.retention.expires_unfrozen, "{:?}", admin.retention);
    assert!(!admin.retention.unfinished_past_event);
    assert!(raffle.erase_at > app.clock.now());

    app.clock.advance(8 * DAY);
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert!(admin.retention.past_erase_date && admin.retention.expires_unfrozen);
    // The date can still move: nobody has joined.
    assert!(admin.actions.edit_rules);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.expired, 1);
    let admin = service::admin(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(admin.view.state, RaffleState::Purged);
    assert!(!admin.retention.expires_unfrozen, "nothing left to expire");
}

/// In an organization, members run raffles, managers (the custom role or
/// permission) also create and configure them, and admins also accept the
/// terms and remove the organization's data. A personal account does
/// everything.
#[tokio::test]
async fn organization_tiers_gate_the_admin_api() {
    use dioxus_clerk::testing::TestClerk;
    let app = app().await;
    let clerk = TestClerk::new().unwrap();
    let public = std::env::temp_dir().join(format!("rafi-public-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(public.join("participant-dist")).unwrap();
    std::fs::write(public.join("index.html"), "<!doctype html>").unwrap();
    let router = super::build_router(
        app.clone(),
        Some(clerk.layer().unwrap()),
        &super::Dirs {
            public: public.clone(),
            participant: public.join("participant-dist"),
        },
        None,
    );
    let call = async |cookie: &str, name: &str, args: &str| -> (StatusCode, String) {
        let mut req = admin_call(name, args);
        req.headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        let (status, _, body) = send(&router, req).await;
        (status, body)
    };
    let org = |user: &str, role: &str| {
        clerk
            .cookie_for(
                &clerk
                    .session(user)
                    .with_organization("org_cnb")
                    .with_organization_role(role),
            )
            .unwrap()
    };
    let member = org("user_member", "org:member");
    let manager = org("user_manager", "org:manager");
    let admin = org("user_admin", "org:admin");
    let by_permission = clerk
        .cookie_for(
            &clerk
                .session("user_perm")
                .with_organization("org_cnb")
                .with_organization_role("org:member")
                .with_organization_permissions(["org:raffles:manage"]),
        )
        .unwrap();
    let create = r#"{"title":"Org raffle","event_date":"2026-10-28","event_ends_at":null}"#;

    // Who they are.
    let (_, body) = call(&member, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(
        (me.owner_id.as_str(), me.access, me.shared),
        ("org_cnb", rafi_api::Access::Member, true)
    );
    let (_, body) = call(&by_permission, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(
        me.access,
        rafi_api::Access::Manager,
        "the permission makes a manager"
    );
    let (_, body) = call(&admin, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!((me.access, me.shared), (rafi_api::Access::Admin, true));

    // Members run raffles; managers also create them.
    let (status, body) = call(&member, "create_raffle", create).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("admin or manager"), "{body}");
    let (status, body) = call(&manager, "create_raffle", create).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id: String = serde_json::from_str(&body).unwrap();
    let (status, body) = call(&member, "save_prize", &format!(r#"{{"id":"{id}","prize":{{"id":null,"name":"Book","sponsor":"","description":"","qty":1,"position":0}}}}"#)).await;
    assert_eq!(status, StatusCode::OK, "a member sets up prizes: {body}");
    let (status, body) = call(
        &member,
        "set_lifecycle",
        &format!(r#"{{"id":"{id}","action":"publish"}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "a member runs the raffle: {body}");
    // The audit log tells the members apart: the owner is the organization,
    // the user is whoever acted.
    let who: Vec<(String, String, String)> = app
        .db
        .query("SELECT actor, user_id, action FROM audit WHERE raffle_id = ? ORDER BY id")
        .bind(&id)
        .all()
        .await
        .unwrap();
    assert_eq!(
        who,
        [
            ("org_cnb".into(), "user_manager".into(), "create".into()),
            ("org_cnb".into(), "user_member".into(), "ready".into()),
        ]
    );
    let (status, body) = call(&member, "delete_raffle", &format!(r#"{{"id":"{id}"}}"#)).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = call(&member, "remove_my_data", "").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body) = call(&manager, "remove_my_data", "").await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a manager isn't an admin: {body}"
    );
    assert!(
        body.contains("Only an organization admin can do this."),
        "{body}"
    );
    assert!(
        raffles::get(&app.db, &id).await.unwrap().is_some(),
        "nothing happened"
    );

    // A personal account is its own admin.
    let person = clerk.cookie("user_solo").unwrap();
    let (_, body) = call(&person, "whoami", "").await;
    let me: rafi_api::Me = serde_json::from_str(&body).unwrap();
    assert_eq!((me.access, me.shared), (rafi_api::Access::Admin, false));
    std::fs::remove_dir_all(&public).ok();
}

/// A frozen raffle nobody ever drew from: erased on the date where it stands,
/// finished by the engine at the ceiling, purged on the run after.
#[tokio::test]
async fn a_frozen_raffle_is_erased_in_place_and_finished_at_the_ceiling() {
    let (recording, live) = recording();
    let app = app_with_live(Config::for_tests(), live).await;
    let hold = app.policy.retention.winner_hold_secs();
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    for name in ["Anna", "Bence"] {
        service::add_manual(&app, ACTOR, &raffle.id, name, Some(format!("{name}@x.org")))
            .await
            .unwrap();
    }
    service::start_draw(&app, ACTOR, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.raffle.opened()).await;
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Frozen
    );

    app.clock.advance(10 * DAY);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((report.erased, report.purged), (2, 0), "{report:?}");
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Frozen, "erased where it stands");
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );
    assert!(recording.notified().contains(&RaffleId::new(&raffle.id)));

    app.clock.set(raffle.erase_at + hold + 1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(
        (report.erased, report.purged),
        (0, 0),
        "nothing left to erase; the engine is asked to finish"
    );
    wait_for(&app, &raffle.id, |r| {
        r.raffle.state == RaffleState::Finished
    })
    .await;
    app.clock.advance(1);
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.purged, 1);
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    assert!(recording.ended().contains(&RaffleId::new(&raffle.id)));
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(
        rafi_raffle::verify(
            &data.manifest,
            &data.manifest_hash,
            &data.events,
            rafi_raffle::drand::resolve_chain
        )
        .ok(),
        "a raffle with no draws still verifies"
    );
}

/// The notice without the optional fields: the contact email is the privacy
/// contact, there is no section from the organizer; and a public raffle
/// without an organizer names "the raffle organizer".
#[tokio::test]
async fn the_notice_without_the_optional_fields() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("Privacy requests: hello@example.com"),
        "{notice}"
    );
    assert!(!notice.contains("From the organizer"), "{notice}");
    assert!(!notice.contains("has since left"));

    // Published but nameless: it can't open, but its notice is public.
    let id = service::create(&app, ACTOR, "Nameless", &service::EventDate::today(&app))
        .await
        .unwrap();
    service::lifecycle(&app, ACTOR, &id, LifecycleCommand::Publish)
        .await
        .unwrap();
    let nameless = raffles::get(&app.db, &id).await.unwrap().unwrap();
    let (status, _, notice) = send(
        &router,
        Request::get(format!("/privacy/{}", nameless.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        notice.contains("The raffle organizer is the data controller for this raffle."),
        "{notice}"
    );
    assert!(!notice.contains("Privacy requests:"));
}

/// Over HTTP: a locked setting is a 409 that names the field; an event date
/// over a year ahead is a 400; registration can't open once the erasure date
/// has passed.
#[tokio::test]
async fn settings_refusals_over_http() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    // `setup_open` works as OWNER; dev auth is `dev`, so hand it over.
    app.db
        .query("UPDATE raffles SET owner_id = 'dev' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    service::add_manual(&app, service::Actor::solo("dev"), &raffle.id, "Anna", None)
        .await
        .unwrap();

    let settings = |event_date: &str, retention_days: i64| {
        let mut s = service::settings_of(&raffle);
        s.event_date = event_date.into();
        s.event_ends_at = None;
        s.retention_days = retention_days;
        format!(
            r#"{{"id":"{}","settings":{}}}"#,
            raffle.id,
            serde_json::to_string(&s).unwrap()
        )
    };
    let (status, _, body) = send(
        &router,
        admin_call("save_settings", &settings("2026-12-24", 7)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let err: rafi_api::admin::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.code, "settings_fixed");
    assert_eq!(
        err.message,
        "The event date cannot change: entrants have already accepted the rules they were shown with."
    );

    let fresh = create(&app, "dev", "Far").await;
    let far = raffles::get(&app.db, &fresh).await.unwrap().unwrap();
    let mut s = service::settings_of(&far);
    s.event_date = "2099-12-31".into();
    s.event_ends_at = None;
    let (status, _, body) = send(
        &router,
        admin_call(
            "save_settings",
            &format!(
                r#"{{"id":"{fresh}","settings":{}}}"#,
                serde_json::to_string(&s).unwrap()
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("at most 366 days ahead"), "{body}");

    // Past its erasure date, a never-frozen raffle can't open.
    let stale = create(&app, "dev", "Stale").await;
    app.clock.advance(10 * DAY);
    service::lifecycle(
        &app,
        service::Actor::solo("dev"),
        &stale,
        LifecycleCommand::Publish,
    )
    .await
    .unwrap();
    let (status, _, body) = send(
        &router,
        admin_call(
            "set_lifecycle",
            &format!(r#"{{"id":"{stale}","action":"open"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.contains("expires within the hour"), "{body}");
}

/// Leaving with nothing but defaults: they go, nothing else is touched.
#[tokio::test]
async fn leaving_with_no_raffles() {
    let app = app().await;
    service::save_defaults(
        &app,
        ACTOR,
        rafi_api::RaffleDefaults {
            organizer_name: "Org".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let other = setup_open(&app, &[("Book", 1)]).await;
    app.db
        .query("UPDATE raffles SET owner_id = 'someone-else' WHERE id = ?")
        .bind(&other.id)
        .run()
        .await
        .unwrap();
    service::remove_owner_data(&app, ACTOR).await.unwrap();
    assert_eq!(
        service::get_defaults(&app, OWNER).await.unwrap(),
        Default::default()
    );
    assert_eq!(
        raffles::get(&app.db, &other.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Open,
        "another owner's raffle is untouched"
    );
}

/// A raffle nobody froze expires on its erasure date, whatever its state:
/// its entrants are deleted outright, it is purged, audited with the count,
/// and its channel ended. One whose date hasn't come is untouched. Expiry
/// logs no erasures: the next run would expire a restored raffle again.
#[tokio::test]
async fn never_frozen_raffles_expire() {
    let (recording, live) = recording();
    let app = app_with_live(Config::for_tests(), live).await;
    let open = setup_open(&app, &[("Cap", 1)]).await;
    service::add_manual(&app, ACTOR, &open.id, "Gabi", Some("gabi@x.org".into()))
        .await
        .unwrap();
    let draft = create(&app, OWNER, "Draft").await;
    // Ten days on, a fresh one.
    app.clock.advance(10 * DAY);
    let fresh = setup_open(&app, &[("Hat", 1)]).await;
    service::add_manual(&app, ACTOR, &fresh.id, "Hanna", None)
        .await
        .unwrap();

    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((report.expired, report.erased), (2, 0), "{report:?}");
    assert!(
        recording.ended().contains(&RaffleId::new(&open.id)),
        "an expired raffle's channel is destroyed"
    );
    for id in [&open.id, &draft] {
        assert!(
            entrants::list(&app.db, id).await.unwrap().is_empty(),
            "deleted outright"
        );
        let r = raffles::get(&app.db, id).await.unwrap().unwrap();
        assert_eq!(r.state, RaffleState::Purged);
        assert!(r.purged_at.is_some() && r.live_ended_at.is_some());
    }
    assert_eq!(
        audit_of(&app, &open.id).await.last().unwrap(),
        &("expire".to_owned(), "1".to_owned())
    );
    assert_eq!(
        audit_of(&app, &draft).await.last().unwrap(),
        &("expire".to_owned(), "0".to_owned())
    );
    let (logged,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = ?")
        .bind(&open.id)
        .one()
        .await
        .unwrap();
    assert_eq!(logged, 0, "expiry logs no erasures");
    assert!(
        entrants::list(&app.db, &fresh.id)
            .await
            .unwrap()
            .iter()
            .all(|e| !e.erased),
        "not due: untouched"
    );
    let again = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(again.expired, 0, "idempotent");

    // The admin of an expired raffle says so, and verify.json has nothing:
    // there was never a manifest.
    let admin = service::admin(&app, OWNER, &open.id).await.unwrap();
    assert_eq!(admin.view.state, RaffleState::Purged);
    assert!(admin.view.manifest_hash.is_none());
    assert!(matches!(
        service::verify_data(&app, &open.slug).await,
        Err(service::Error::NotFound)
    ));
}

/// A raffle can be deleted until its first draw is announced: a freeze by
/// accident has a way out, and a raffle that drew keeps its record. The
/// admin is told which, and a frozen raffle's engine state goes with it.
#[tokio::test]
async fn deleting_stops_at_the_first_draw() {
    let app = app().await;
    let router = routes(app.clone());

    // Frozen by accident: deletable, and the engine forgets it.
    let frozen = setup_open(&app, &[("Book", 1)]).await;
    join(&router, &frozen.join_code, "Anna", "anna@example.com").await;
    service::start_draw(&app, ACTOR, &frozen.id).await.unwrap();
    wait_for(&app, &frozen.id, |r| r.raffle.opened()).await;
    assert!(
        service::admin(&app, OWNER, &frozen.id)
            .await
            .unwrap()
            .actions
            .delete
    );
    service::delete(&app, ACTOR, &frozen.id).await.unwrap();
    assert!(raffles::get(&app.db, &frozen.id).await.unwrap().is_none());
    let engine = app.engine.as_ref().unwrap();
    let forgotten = engine
        .decide(
            &RaffleId::new(&frozen.id),
            &Decision::Finish,
            "after-delete",
        )
        .await;
    assert!(
        forgotten.is_err(),
        "the engine was asked to finish and forgot the raffle: {forgotten:?}"
    );

    // Drawn: the record stays; the admin offers no delete, and a call refuses.
    let drawn = setup_open(&app, &[("Mug", 1)]).await;
    join(&router, &drawn.join_code, "Bence", "bence@example.com").await;
    service::start_draw(&app, ACTOR, &drawn.id).await.unwrap();
    let state = wait_for(&app, &drawn.id, |r| r.raffle.opened())
        .await
        .drawn
        .unwrap();
    let item = state.items()[0].id.clone();
    service::command(
        &app,
        ACTOR,
        &drawn.id,
        DrawRequest { item: item.clone() },
        "d1",
    )
    .await
    .unwrap();
    wait_for(&app, &drawn.id, |r| {
        r.drawn
            .as_ref()
            .is_some_and(|s| s.items()[0].status() == ItemStatus::Drawn)
    })
    .await;
    assert!(
        !service::admin(&app, OWNER, &drawn.id)
            .await
            .unwrap()
            .actions
            .delete
    );
    let refused = service::delete(&app, ACTOR, &drawn.id).await;
    assert!(
        matches!(
            refused,
            Err(service::Error::Refused(service::Refusal::Drawn))
        ),
        "{refused:?}"
    );
    assert_eq!(service::Refusal::Drawn.code(), "drawn");
    assert!(
        service::Refusal::Drawn
            .to_string()
            .contains("stays verifiable")
    );
    // Finished with the one draw, still not deletable: purge is the way.
    service::command(
        &app,
        ACTOR,
        &drawn.id,
        Decision::Resolve {
            item,
            resolution: Resolution::Claimed,
        },
        "r1",
    )
    .await
    .unwrap();
    service::command(&app, ACTOR, &drawn.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let admin = service::admin(&app, OWNER, &drawn.id).await.unwrap();
    assert!(!admin.actions.delete && admin.actions.purge);
}

/// Once a raffle's erasure date has passed, nobody joins it, even while it is
/// still open (it expires within the hour): the form would promise erasure
/// on a day gone by. The admin is warned a day ahead.
#[tokio::test]
async fn nobody_joins_past_the_erasure_date() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    // A period of 0: erased at midnight, and a day passes.
    let raffle = retain(&app, &raffle.id, 0).await;
    app.clock.advance(DAY);
    assert!(raffle.past_erase_date(app.clock.now()));
    assert_eq!(
        raffle.state,
        RaffleState::Open,
        "still open until housekeeping runs"
    );

    let (status, cookie, body) = join(&router, &raffle.join_code, "Anna", "anna@example.com").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(cookie.is_none());
    assert!(body.contains("Registration is closed."), "{body}");
    assert!(
        entrants::list(&app.db, &raffle.id)
            .await
            .unwrap()
            .is_empty()
    );
    // The join page says so too, with no form.
    let (status, _, page) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!page.contains("name=\"email\""), "no form: {page}");

    // A day ahead, the admin is warned; past the date, told it has passed.
    let soon = setup_open(&app, &[("Book", 1)]).await;
    retain(&app, &soon.id, 0).await;
    let r = service::admin(&app, OWNER, &soon.id)
        .await
        .unwrap()
        .retention;
    assert!(r.erasure_imminent && !r.past_erase_date, "{r:?}");
    let r = service::admin(&app, OWNER, &raffle.id)
        .await
        .unwrap()
        .retention;
    assert!(!r.erasure_imminent && r.past_erase_date, "{r:?}");
}

/// A frozen raffle the engine doesn't hold (its freeze stopped before the
/// engine opened it, and the organizer walked away) is purged at the
/// ceiling instead of being nudged every hour for good; one frozen without
/// a manifest expires on the date like a never-frozen raffle.
#[tokio::test]
async fn abandoned_freezes_leave_limbo() {
    let app = app().await;
    let hold = app.cfg.retention.winner_hold_days * DAY;

    // A manifest written, never handed to the engine (as `Start::Resend`
    // would find it): everyone is a non-winner, so all are erased on the
    // date, and at the ceiling the engine refuses to finish what it never
    // opened, which purges the raffle here.
    let orphan = setup_open(&app, &[("Book", 1)]).await;
    let orphan = retain(&app, &orphan.id, 0).await;
    service::add_manual(&app, ACTOR, &orphan.id, "Anna", None)
        .await
        .unwrap();
    assert!(
        raffles::freeze(&app.db, &orphan.id, RaffleState::Open, app.clock.now())
            .await
            .unwrap()
    );
    let manifest = rafi_raffle::Manifest::new(
        orphan.id.as_str(),
        rafi_raffle::BeaconSource::Drand(app.chain),
        entrants::active_ids(&app.db, &orphan.id)
            .await
            .unwrap()
            .into_iter()
            .map(Into::into)
            .collect(),
        vec![rafi_raffle::ManifestItem {
            id: "book-1".into(),
            name: "Book".into(),
        }],
    );
    raffles::set_manifest(
        &app.db,
        &orphan.id,
        &manifest.canonical_json(),
        &manifest.hash().to_string(),
        app.clock.now(),
    )
    .await
    .unwrap();
    let r = raffles::get(&app.db, &orphan.id).await.unwrap().unwrap();
    assert!(r.state == RaffleState::Frozen && r.manifest.is_some() && !r.opened());

    // Frozen without a manifest: the freeze stopped after closing registration.
    let stuck = setup_open(&app, &[("Book", 1)]).await;
    let stuck = retain(&app, &stuck.id, 0).await;
    service::add_manual(&app, ACTOR, &stuck.id, "Bence", None)
        .await
        .unwrap();
    assert!(
        raffles::freeze(&app.db, &stuck.id, RaffleState::Open, app.clock.now())
            .await
            .unwrap()
    );

    app.clock.advance(10 * DAY);
    assert!(
        service::admin(&app, OWNER, &stuck.id)
            .await
            .unwrap()
            .retention
            .expires_unfrozen,
        "the admin says it expires"
    );
    let report = service::housekeeping::run(&app).await.unwrap();
    assert_eq!(report.expired, 1, "the manifest-less freeze expired");
    assert_eq!(
        raffles::get(&app.db, &stuck.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Purged
    );
    assert!(entrants::list(&app.db, &stuck.id).await.unwrap().is_empty());
    assert_eq!(
        raffles::get(&app.db, &orphan.id)
            .await
            .unwrap()
            .unwrap()
            .state,
        RaffleState::Frozen,
        "the one with a manifest is erased, not moved, before the ceiling"
    );
    assert!(
        entrants::list(&app.db, &orphan.id)
            .await
            .unwrap()
            .iter()
            .all(|e| e.erased)
    );

    app.clock.set(orphan.erase_at + hold + 1);
    service::housekeeping::run(&app).await.unwrap();
    let r = raffles::get(&app.db, &orphan.id).await.unwrap().unwrap();
    assert_eq!(
        r.state,
        RaffleState::Purged,
        "purged, the engine not holding it"
    );
    assert!(r.purged_at.is_some() && r.live_ended_at.is_some());
    assert_eq!(
        audit_of(&app, &orphan.id).await.last().unwrap(),
        &("auto_purge".to_owned(), "unfinished".to_owned())
    );
    app.clock.advance(1);
    let again = service::housekeeping::run(&app).await.unwrap();
    assert_eq!((again.expired, again.purged), (0, 0), "nothing left to do");
}
