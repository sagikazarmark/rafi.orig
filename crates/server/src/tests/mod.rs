//! Integration tests: in-memory SQLite (or, with `RAFI_TEST_POSTGRES_URL`
//! set, a fresh Postgres schema per test), the in-process raffle engine
//! (`rafi_engine::local`), mock drand, and a test's clock: it says the time
//! the test started until the test moves it (`app.clock`), and the engine's
//! writes follow it too. Drand's rounds are real time all the same.
//!
//! This module holds the fixtures the tests share; the tests are in its
//! submodules, one per concern.

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

mod admin_auth;
mod draws;
mod flow;
mod headers;
mod joins;
mod live;
mod people;
mod privacy;
mod public;
mod retention;
mod settings;
mod site;

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

async fn get_json(
    router: &Router,
    url: &str,
) -> (StatusCode, header::HeaderMap, serde_json::Value) {
    let (status, headers, body) =
        send(router, Request::get(url).body(Body::empty()).unwrap()).await;
    let json = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
    (status, headers, json)
}

/// Adds `name` (and `email`) as `actor` with the one-person form, in a
/// batch of its own; returns their entrant ID.
pub(crate) async fn add_one(
    app: &service::App,
    actor: service::Actor<'_>,
    raffle_id: &str,
    name: &str,
    email: Option<String>,
) -> service::Result<String> {
    let add = rafi_api::AddRequest {
        batch_id: service::ids::id(),
        origin: rafi_api::Origin::One,
        rows: vec![rafi_api::AddRow {
            name: name.into(),
            email,
            ..rafi_api::AddRow::default()
        }],
        name_format: None,
    };
    let result = service::add_entrants(app, actor, raffle_id, add).await?;
    Ok(result
        .added
        .first()
        .map(|a| a.id.clone())
        .unwrap_or_default())
}

/// POSTs `args` (JSON) to the admin function `name`.
fn admin_call(name: &str, args: &str) -> Request<Body> {
    Request::post(format!("/api/admin/{name}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(args.to_owned()))
        .unwrap()
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

/// A raffle with three entrants, frozen and finished without draws.
async fn finished_raffle(app: &AppState) -> RaffleRow {
    let raffle = setup_open(app, &[("Prize", 1)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        add_one(app, ACTOR, &raffle.id, name, None).await.unwrap();
    }
    service::start_draw(app, ACTOR, &raffle.id).await.unwrap();
    service::command(app, ACTOR, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state, RaffleState::Finished);
    r
}

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
    draw_and_claim_as(app, ACTOR, raffle_id, item).await
}

/// [`draw_and_claim`], as `actor`.
async fn draw_and_claim_as(
    app: &AppState,
    actor: service::Actor<'_>,
    raffle_id: &str,
    item: &rafi_raffle::ItemId,
) -> String {
    service::command(
        app,
        actor,
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
        actor,
        raffle_id,
        Decision::Resolve {
            k: None,
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

/// An open raffle of `dev`'s (the organizer under dev auth), and the routes.
async fn dev_raffle(cfg: Config) -> (AppState, Router, RaffleRow) {
    dev_raffle_with(cfg, &[("Book", 1)]).await
}

/// [`dev_raffle`], with `prizes`.
async fn dev_raffle_with(cfg: Config, prizes: &[(&str, u32)]) -> (AppState, Router, RaffleRow) {
    let mut cfg = cfg;
    cfg.dev_auth = true;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, prizes).await;
    app.db
        .query("UPDATE raffles SET owner_id = 'dev' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    (app, router, raffle)
}

/// The arguments of `add_entrants` for raffle `id`: an add of `rows` from
/// `origin` (`one` or `list`), batch `batch`.
fn add_args(id: &str, batch: &str, origin: &str, rows: serde_json::Value) -> String {
    serde_json::json!({ "id": id, "add": { "batch_id": batch, "origin": origin, "rows": rows } })
        .to_string()
}
