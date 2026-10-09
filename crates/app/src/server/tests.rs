//! Integration tests: in-memory SQLite (or, with `RAFI_TEST_POSTGRES_URL`
//! set, a fresh Postgres schema per test), the in-process drawing engine
//! (`rafi_engine_local`), mock drand.

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use rafi_core::raffle::RaffleState;
use rafi_drawing::{ItemStatus, Resolution};
use rafi_live::LiveTransport;
use rafi_platform::{Limiter, Notifier, Polling, Recording};
use rafi_store::{RaffleRow, entrants, prizes, raffles};
use tower::ServiceExt;

use rafi_drawing::{Decision, DrawRequest};

use super::{AppState, config::Config, routes, service};
use crate::model::{Lifecycle, PrizeInput};

const OWNER: &str = "user_test";

async fn app() -> AppState {
    app_with(Config::for_tests()).await
}

async fn app_with(cfg: Config) -> AppState {
    app_with_live(cfg, Arc::new(Polling::default())).await
}

/// The drawing engine's drand: the mock chain, no lead.
fn mock_drand() -> rafi_drand::DrandConfig {
    rafi_drand::DrandConfig {
        relays: vec![],
        mock: true,
        lead_secs: 0,
        timeout_secs: 5,
    }
}

async fn app_with_live(cfg: Config, live: Arc<dyn Notifier>) -> AppState {
    app_with_engine(cfg, mock_drand(), live).await
}

/// An app whose drawing engine is the in-process one (drawing with `drand`),
/// writing into the same database. With `RAFI_TEST_POSTGRES_URL` set, the
/// test database is a fresh schema there instead of in-memory SQLite.
async fn app_with_engine(
    mut cfg: Config,
    drand: rafi_drand::DrandConfig,
    live: Arc<dyn Notifier>,
) -> AppState {
    if cfg.database_url == Config::for_tests().database_url
        && let Some(url) = rafi_store::testing::postgres_url().await
    {
        cfg.database_url = url;
    }
    let db = rafi_store::Db::connect(&cfg.database_url)
        .await
        .expect("database");
    let purge =
        rafi_store::PurgeAfter::days(cfg.retention_nonwinners_days, cfg.retention_winners_days);
    let engine = rafi_engine_local::LocalEngine::new(
        rafi_drand::DrandClient::new(drand).expect("drand configuration"),
        rafi_store::DrawWriter::new(db.clone(), purge),
        Polling::default(),
    );
    AppState::from_parts(
        cfg,
        db,
        Some(Arc::new(engine)),
        live,
        Arc::new(Limiter::default()),
    )
    .expect("app state")
}

async fn send(router: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    let resp = router.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

async fn solved_altcha(router: &Router) -> String {
    let (_, _, body) = send(
        router,
        Request::get("/api/altcha").body(Body::empty()).unwrap(),
    )
    .await;
    let challenge = serde_json::from_str(&body).expect("challenge json");
    rafi_live::altcha::solve(&challenge)
        .expect("solvable")
        .to_payload()
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
    let altcha = solved_altcha(router).await;
    let body = form(&[
        ("name", name),
        ("email", email),
        ("rules", "on"),
        ("altcha", &altcha),
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

async fn setup_open(app: &AppState, prizes: &[(&str, u32)]) -> RaffleRow {
    let id = service::create(app, OWNER, "Cloud Native Budapest")
        .await
        .unwrap();
    for (i, (name, qty)) in prizes.iter().enumerate() {
        service::upsert_prize(
            app,
            OWNER,
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
    service::lifecycle(app, OWNER, &id, Lifecycle::Publish)
        .await
        .unwrap();
    service::lifecycle(app, OWNER, &id, Lifecycle::Open)
        .await
        .unwrap();
    raffles::get(&app.db, &id).await.unwrap().unwrap()
}

async fn wait_for<F: Fn(&RaffleRow) -> bool>(app: &AppState, id: &str, f: F) -> RaffleRow {
    for _ in 0..200 {
        let r = raffles::get(&app.db, id).await.unwrap().unwrap();
        if f(&r) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("condition not met in time");
}

/// [`wait_for`] for slow conditions (stalls take a round period plus the timeout).
async fn wait_long<F: Fn(&RaffleRow) -> bool>(app: &AppState, id: &str, f: F) -> RaffleRow {
    for _ in 0..1000 {
        let r = raffles::get(&app.db, id).await.unwrap().unwrap();
        if f(&r) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("condition not met in time");
}

/// An app whose drand relay is unreachable: every draw stalls on its round
/// (quicknet, 3 s lead, gives up 1 s after the round is due).
async fn unreachable_drand_app() -> AppState {
    let mut cfg = Config::for_tests();
    cfg.drand_chain = rafi_drawing::drand::chains::QUICKNET_HASH.to_string();
    let drand = rafi_drand::DrandConfig {
        relays: vec!["http://127.0.0.1:9".into()],
        mock: false,
        lead_secs: 3,
        timeout_secs: 1,
    };
    app_with_engine(cfg, drand, Arc::new(Polling::default())).await
}

#[tokio::test]
async fn full_raffle_flow() {
    let mut cfg = Config::for_tests();
    cfg.retention_enabled = true;
    let app = app_with(cfg).await;
    let router = routes(app.clone());
    let raffle = setup_open(
        &app,
        &[("KCD Budapest ticket", 2), ("CNCF exam voucher", 1)],
    )
    .await;
    let code = raffle.join_code.clone();

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
        page.contains(&me.claim_code),
        "claim code is shown to the entrant"
    );

    // Same email (normalized) on another device: already in, no new entry, no cookie.
    let (status, cookie, page) = join(&router, &code, "Someone", "P0+other@Example.com").await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_none());
    assert!(page.contains("already entered"));

    // A person without a phone.
    service::add_manual(&app, OWNER, &raffle.id, "No Phone", None)
        .await
        .unwrap();
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 6);

    // Freeze; joining is closed afterwards.
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.drawing.is_some()).await;
    assert_eq!(r.state(), RaffleState::Frozen);
    let (status, cookie, _) = join(&router, &code, "Late", "late@example.com").await;
    assert_ne!(status, StatusCode::SEE_OTHER);
    assert!(cookie.is_none());
    assert!(
        service::add_manual(&app, OWNER, &raffle.id, "Late", None)
            .await
            .is_err()
    );
    assert!(
        service::upsert_prize(
            &app,
            OWNER,
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
    let state = r.drawing().unwrap().unwrap();
    let items: Vec<_> = state.items().iter().map(|u| u.id.clone()).collect();
    let mut winners = Vec::new();
    for (i, item) in items.iter().enumerate() {
        service::command(
            &app,
            OWNER,
            &raffle.id,
            DrawRequest { item: item.clone() },
            &format!("c{i}"),
        )
        .await
        .unwrap();
        let r = wait_for(&app, &raffle.id, |r| {
            r.drawing()
                .unwrap()
                .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Drawn)
        })
        .await;
        if i == 0 {
            let first = r
                .drawing()
                .unwrap()
                .unwrap()
                .item(item)
                .unwrap()
                .winner()
                .cloned()
                .unwrap();
            service::command(
                &app,
                OWNER,
                &raffle.id,
                Decision::Resolve {
                    item: item.clone(),
                    outcome: Resolution::NotHere,
                },
                "nh",
            )
            .await
            .unwrap();
            service::command(
                &app,
                OWNER,
                &raffle.id,
                DrawRequest { item: item.clone() },
                "redraw",
            )
            .await
            .unwrap();
            let r = wait_for(&app, &raffle.id, |r| {
                r.drawing()
                    .unwrap()
                    .is_some_and(|s| s.item(item).unwrap().status() == ItemStatus::Drawn)
            })
            .await;
            assert_ne!(
                r.drawing()
                    .unwrap()
                    .unwrap()
                    .item(item)
                    .unwrap()
                    .winner()
                    .cloned()
                    .unwrap(),
                first
            );
        }
        let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
        winners.push(
            r.drawing()
                .unwrap()
                .unwrap()
                .item(item)
                .unwrap()
                .winner()
                .cloned()
                .unwrap(),
        );
        service::command(
            &app,
            OWNER,
            &raffle.id,
            Decision::Resolve {
                item: item.clone(),
                outcome: Resolution::Claimed,
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
            OWNER,
            &raffle.id,
            DrawRequest {
                item: items[0].clone(),
            },
            "x"
        )
        .await
        .is_err()
    );

    service::command(&app, OWNER, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state(), RaffleState::Finished);
    assert!(r.purge_nonwinners_at.is_some());

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
    let report = rafi_drawing::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), items.len() + 1);

    // CSV export lists the winners (and the not-here entrant).
    let csv = service::export_csv(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(csv.lines().count(), 1 + items.len() + 1);

    // Retention: once due, personal data is erased but the draw stays verifiable.
    app.db
        .query("UPDATE raffles SET purge_nonwinners_at = 0, purge_winners_at = 0 WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    let report = super::retention::run(&app).await.unwrap();
    assert_eq!(report.erased, 6);
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert!(
        entrants
            .iter()
            .all(|e| e.purged && e.email_raw.is_none() && e.token_hash.is_none())
    );
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state(),
        RaffleState::Purged
    );
    // Each erasure is logged once, by ID; a second run has nothing to do.
    let logged: Vec<(String, String)> = app
        .db
        .query("SELECT raffle_id, entrant_id FROM erasures ORDER BY entrant_id")
        .all()
        .await
        .unwrap();
    let mut ids: Vec<(String, String)> = entrants
        .iter()
        .map(|e| (raffle.id.clone(), e.id.clone()))
        .collect();
    ids.sort();
    assert_eq!(logged, ids);
    assert_eq!(super::retention::run(&app).await.unwrap().erased, 0);
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(
        rafi_drawing::verify(
            &data.manifest,
            &data.manifest_hash,
            &data.events,
            rafi_drand::resolve_chain
        )
        .ok()
    );
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
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    wait_for(&app, &raffle.id, |r| r.drawing.is_some()).await;
    let state = raffles::get(&app.db, &raffle.id)
        .await
        .unwrap()
        .unwrap()
        .drawing()
        .unwrap()
        .unwrap();
    service::command(
        &app,
        OWNER,
        &raffle.id,
        DrawRequest {
            item: state.items()[0].id.clone(),
        },
        "c",
    )
    .await
    .unwrap();
    wait_for(&app, &raffle.id, |r| {
        r.drawing()
            .unwrap()
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
        .clone();
    assert!(body.contains("Visible"));
    assert!(!body.contains("secret-address"), "email leaked: {body}");
    assert!(!body.contains(&format!("\"{claim}\"")), "claim code leaked");
    let (_, _, verify) = send(
        &router,
        Request::get(format!("/api/r/{}/verify.json", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert!(!verify.contains("secret-address"));
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
        view["live"],
        serde_json::json!({"kind": "poll", "every_ms": 2500}),
        "no push natively: clients poll"
    );

    // Polling again without a write: the same version.
    let (_, _, again) = get_json(&router, &url).await;
    assert_eq!(again["v"].as_u64().unwrap(), v0);

    service::add_manual(&app, OWNER, &raffle.id, "Anna", None)
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

    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.drawing.is_some()).await;
    let item = r.drawing().unwrap().unwrap().items()[0].id.clone();
    service::command(
        &app,
        OWNER,
        &raffle.id,
        DrawRequest { item: item.clone() },
        "c",
    )
    .await
    .unwrap();
    let r = wait_for(&app, &raffle.id, |r| {
        r.drawing()
            .unwrap()
            .is_some_and(|s| s.item(&item).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    let winner_id = r
        .drawing()
        .unwrap()
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
            !text.contains(&format!("\"{}\"", e.claim_code)),
            "a claim code: {text}"
        );
    }

    // A hidden winner's name is masked.
    service::patch_entrant(
        &app,
        OWNER,
        &raffle.id,
        winner_id.as_str(),
        crate::model::EntrantPatch {
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
    assert!(page.contains(r#""live":{"kind":"poll","every_ms":2500}"#));
    assert!(
        !page.contains(&other.display_name),
        "a non-winner's name on the page"
    );
}

#[tokio::test]
async fn drafts_are_private() {
    let app = app().await;
    let router = routes(app.clone());
    let id = service::create(&app, OWNER, "Secret").await.unwrap();
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
        Err(service::AppError::NotFound)
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
    let altcha = solved_altcha(&router).await;
    let (status, _, page) = send(
        &router,
        post(form(&[
            ("name", "Ann"),
            ("email", "ann@example.com"),
            ("altcha", &altcha),
        ])),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("accept the rules"));
    let altcha = solved_altcha(&router).await;
    let (status, _, _) = send(
        &router,
        post(form(&[
            ("name", "fasz"),
            ("email", "x@example.com"),
            ("rules", "on"),
            ("altcha", &altcha),
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
    cfg.drand_chain = rafi_drawing::drand::chains::QUICKNET_HASH.to_string();
    let app = app_with_engine(cfg, mock_drand(), Arc::new(Polling::default())).await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    service::add_manual(&app, OWNER, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let err = service::start_draw(&app, OWNER, &raffle.id)
        .await
        .unwrap_err();
    assert!(matches!(err, service::AppError::Conflict(_)), "{err}");
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state(), RaffleState::Open, "nothing was frozen");
    assert!(r.manifest.is_none(), "nothing was committed");
}

/// Without a drawing engine configured, nothing can start.
#[tokio::test]
async fn draws_need_restate() {
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = AppState::new(cfg).await.unwrap();
    assert!(app.engine.is_none());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    service::add_manual(&app, OWNER, &raffle.id, "Anna", None)
        .await
        .unwrap();
    let err = service::start_draw(&app, OWNER, &raffle.id)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Restate"), "{err}");
    assert_eq!(
        raffles::get(&app.db, &raffle.id)
            .await
            .unwrap()
            .unwrap()
            .state(),
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
    // And for the participant scripts.
    let participant = public.join("participant-dist");
    std::fs::create_dir_all(&participant).unwrap();
    std::fs::write(participant.join("fallback.js"), "// fallback").unwrap();
    let router = super::build_router(
        app.clone(),
        Some(clerk.layer().unwrap()),
        &super::Dirs {
            public: public.clone(),
            participant,
        },
    );
    let (status, _, body) = send(
        &router,
        Request::get("/participant/fallback.js")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "// fallback"));

    let (status, _, body) = send(&router, admin_call("whoami", "{}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(body.contains("Please sign in."), "{body}");

    let mut req = admin_call("whoami", "{}");
    req.headers_mut().insert(
        header::COOKIE,
        clerk.cookie("user_2abc").unwrap().parse().unwrap(),
    );
    let (status, _, body) = send(&router, req).await;
    assert!(status.is_success(), "{status}: {body}");
    let me: crate::model::Me = serde_json::from_str(&body).unwrap();
    assert_eq!(me.owner_id, "user_2abc");
    assert!(!me.dev_auth);

    // The web app, with a single-page fallback for its own routes.
    for path in ["/", "/admin", "/admin/r/abc", "/s/some-slug"] {
        let (status, _, body) =
            send(&router, Request::get(path).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains("<div id=\"main\">"), "{path}: {body}");
    }
    std::fs::remove_dir_all(&public).ok();
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
        let err: crate::api::ApiError = serde_json::from_str(&body).unwrap();
        assert_eq!(err.message, "Please sign in.");
    }

    // Dev auth: the organizer is `dev`.
    let mut cfg = Config::for_tests();
    cfg.dev_auth = true;
    let app = AppState::new(cfg).await.unwrap();
    let router = routes(app.clone());

    let (status, _, body) = send(&router, admin_call("whoami", "")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let me: crate::model::Me = serde_json::from_str(&body).unwrap();
    assert_eq!((me.owner_id.as_str(), me.dev_auth), ("dev", true));

    let (status, _, body) = send(
        &router,
        admin_call("create_raffle", r#"{"title":"Meetup #1"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id: String = serde_json::from_str(&body).unwrap();

    let (status, _, body) = send(&router, admin_call("list_raffles", "{}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let list: Vec<crate::model::RaffleSummary> = serde_json::from_str(&body).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "Meetup #1");

    // Unit results are JSON `null`.
    let (status, _, body) = send(
        &router,
        admin_call(
            "lifecycle",
            &format!(r#"{{"id":"{id}","action":"publish"}}"#),
        ),
    )
    .await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "null"));
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state(),
        RaffleState::Ready
    );

    // Errors carry the service's message and a matching status.
    let (status, _, body) = send(&router, admin_call("create_raffle", r#"{"title":" "}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let err: crate::api::ApiError = serde_json::from_str(&body).unwrap();
    assert_eq!(err.message, "The title must be 1–120 characters.");
    let (status, _, body) = send(&router, admin_call("get_raffle", r#"{"id":"nope"}"#)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _, body) = send(&router, admin_call("get_raffle", "{}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _, _) = send(&router, admin_call("no_such_function", "{}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn lifecycle_guards() {
    let app = app().await;
    let id = service::create(&app, OWNER, "Guards").await.unwrap();
    assert!(
        service::lifecycle(&app, OWNER, &id, Lifecycle::Open)
            .await
            .is_err(),
        "cannot open a draft"
    );
    service::lifecycle(&app, OWNER, &id, Lifecycle::Publish)
        .await
        .unwrap();
    service::lifecycle(&app, OWNER, &id, Lifecycle::Open)
        .await
        .unwrap();
    assert!(
        service::start_draw(&app, OWNER, &id).await.is_err(),
        "no prizes"
    );
    service::upsert_prize(
        &app,
        OWNER,
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
        service::start_draw(&app, OWNER, &id).await.is_err(),
        "no entrants"
    );
    // The failed freeze must not leave the raffle frozen.
    assert_eq!(
        raffles::get(&app.db, &id).await.unwrap().unwrap().state(),
        RaffleState::Open
    );
    assert!(
        service::add_fakes(&app, OWNER, &id, 3).await.is_err(),
        "fakes need test mode"
    );
    let dup = service::duplicate(&app, OWNER, &id).await.unwrap();
    assert_eq!(prizes::list(&app.db, &dup).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_freeze_interrupted_before_its_commit_can_still_start() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 2)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        service::add_manual(&app, OWNER, &raffle.id, name, None)
            .await
            .unwrap();
    }
    // The freeze's first write landed, the commit document's did not.
    app.db
        .query("UPDATE raffles SET state = 'frozen' WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();

    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.drawing.is_some()).await;
    let doc: rafi_drawing::Manifest =
        serde_json::from_str(r.manifest.as_deref().expect("commit document")).unwrap();
    assert_eq!(doc.entrants.len(), 3);
    assert_eq!(doc.items.len(), 2);
    assert_eq!(
        r.manifest_hash.as_deref(),
        Some(rafi_drawing::ManifestHash::of(r.manifest.as_deref().unwrap()).to_string()).as_deref()
    );
    assert!(r.frozen_at.is_some());
    assert_eq!(
        r.drawing().unwrap().unwrap().manifest_hash().to_string(),
        r.manifest_hash.unwrap()
    );

    // Starting again re-sends the same commitment.
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let again = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(again.manifest, r.manifest);
}

#[tokio::test]
async fn fake_entrants_are_added_in_one_batch() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Book", 1)]).await;
    let mut settings = service::settings_of(&raffle);
    settings.test_mode = true;
    service::update_settings(&app, OWNER, &raffle.id, settings)
        .await
        .unwrap();
    service::add_fakes(&app, OWNER, &raffle.id, service::MAX_FAKE_ENTRANTS + 1)
        .await
        .unwrap();
    let entrants = entrants::list(&app.db, &raffle.id).await.unwrap();
    assert_eq!(entrants.len(), service::MAX_FAKE_ENTRANTS as usize);
    assert!(entrants.iter().all(|e| e.source == "fake" && !e.hidden));
    let ids: std::collections::HashSet<_> = entrants.iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), entrants.len());
}

/// A draw whose round never arrives in time stalls on that round: retrying
/// keeps the round, and the only exit (abandon) is logged for the verifier.
#[tokio::test]
async fn stalled_draw_keeps_its_round() {
    use rafi_drawing::Event;

    let app = unreachable_drand_app().await;
    let raffle = setup_open(&app, &[("Prize", 3)]).await;
    for i in 0..4 {
        service::add_manual(&app, OWNER, &raffle.id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.drawing.is_some()).await;
    let items: Vec<_> = r
        .drawing()
        .unwrap()
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
        service::command(app, OWNER, raffle_id, DrawRequest { item: item.into() }, id).await
    }
    let draw = |item: &str, id: &str| {
        let (item, id) = (item.to_owned(), id.to_owned());
        let (app, raffle_id) = (app.clone(), raffle.id.clone());
        async move { draw_cmd(&app, &raffle_id, &item, &id).await }
    };
    let stalls = |r: &RaffleRow| {
        r.drawing()
            .unwrap()
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
        .drawing()
        .unwrap()
        .unwrap()
        .pending()
        .cloned()
        .expect("still pending");

    // Another item cannot start; this one retries the same round.
    assert!(draw(items[1].as_str(), "d2").await.is_err());
    draw(items[0].as_str(), "d3").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 2).await;
    let s = r.drawing().unwrap().unwrap();
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
        OWNER,
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
    let next = r.drawing().unwrap().unwrap().pending().cloned().unwrap();
    assert_eq!(next.k, pending.k, "the abandoned draw's number is reused");
    assert!(
        next.round >= pending.round,
        "a new draw commits to a current round"
    );

    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let report = rafi_drawing::verify(
        &data.manifest,
        &data.manifest_hash,
        &data.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    let abandoned: Vec<_> = report.abandoned.iter().map(|c| c.round).collect();
    assert_eq!(abandoned, [pending.round]);
    assert!(rafi_drawing::verify_public(&data.manifest, &data.manifest_hash, &data.events).ok());
}

/// While a draw waits for its round, "not here" is refused; "handed over" is
/// accepted, and the engine applies it once that draw settles.
#[tokio::test]
async fn only_handing_over_passes_a_waiting_draw() {
    let drand = rafi_drand::DrandConfig {
        lead_secs: 3,
        ..mock_drand()
    };
    let app = app_with_engine(Config::for_tests(), drand, Arc::new(Polling::default())).await;
    let raffle = setup_open(&app, &[("Prize", 2)]).await;
    for i in 0..3 {
        service::add_manual(&app, OWNER, &raffle.id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.drawing.is_some()).await;
    let items: Vec<_> = r
        .drawing()
        .unwrap()
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
            service::command(&app, OWNER, &raffle_id, request, &id).await
        }
    };
    let resolve = |outcome, id: &str| {
        let id = id.to_owned();
        let (app, raffle_id, item) = (app.clone(), raffle.id.clone(), items[0].clone());
        async move {
            let decision = Decision::Resolve { item, outcome };
            service::command(&app, OWNER, &raffle_id, decision, &id).await
        }
    };

    draw(items[0].as_str(), "d1").await.unwrap();
    wait_long(&app, &raffle.id, |r| {
        r.drawing()
            .unwrap()
            .is_some_and(|s| s.item(&items[0]).unwrap().status() == ItemStatus::Drawn)
    })
    .await;
    draw(items[1].as_str(), "d2").await.unwrap();
    wait_for(&app, &raffle.id, |r| {
        r.drawing()
            .unwrap()
            .is_some_and(|s| s.pending().as_ref().is_some_and(|p| p.item == items[1]))
    })
    .await;
    assert!(resolve(Resolution::NotHere, "nh").await.is_err());
    resolve(Resolution::Claimed, "cl").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| {
        r.drawing()
            .unwrap()
            .is_some_and(|s| s.item(&items[0]).unwrap().status() == ItemStatus::Claimed)
    })
    .await;
    assert_eq!(
        r.drawing()
            .unwrap()
            .unwrap()
            .item(&items[1])
            .unwrap()
            .status(),
        ItemStatus::Drawn,
        "applied after the draw settled"
    );
}

/// The public page only trusts public drand chains: a mock raffle never passes there.
#[tokio::test]
async fn mock_raffles_are_not_publicly_verified() {
    let app = app().await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    service::add_manual(&app, OWNER, &raffle.id, "Person", None)
        .await
        .unwrap();
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let public = rafi_drawing::verify_public(&data.manifest, &data.manifest_hash, &data.events);
    assert!(public.chain_unknown() && !public.ok());
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

/// A proof-of-work is redeemed once: the second use fails the `altcha_used`
/// primary key. Expired rows leave with the retention run.
#[tokio::test]
async fn proof_of_work_replays_are_refused() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let altcha = solved_altcha(&router).await;
    let post = |name: &str, email: &str| {
        Request::post(format!("/j/{}", raffle.join_code))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form(&[
                ("name", name),
                ("email", email),
                ("rules", "on"),
                ("altcha", &altcha),
            ])))
            .unwrap()
    };
    let (status, _, _) = send(&router, post("Anna", "anna@example.com")).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let (status, _, page) = send(&router, post("Bot", "bot@example.com")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(page.contains("human"), "{page}");
    assert_eq!(entrants::list(&app.db, &raffle.id).await.unwrap().len(), 1);

    let solution = rafi_live::altcha::Solution::from_payload(&altcha).unwrap();
    let rows: Vec<(String, i64)> = app
        .db
        .query("SELECT challenge, expires_at FROM altcha_used")
        .all()
        .await
        .unwrap();
    assert_eq!(
        rows,
        [(
            solution.challenge.clone(),
            solution.expires_at().unwrap() as i64
        )]
    );
    assert!(!super::altcha::redeem(&app, &altcha).await.unwrap());

    // A forged solution never reaches the table.
    let mut forged = solution.clone();
    forged.signature = "00".repeat(32);
    assert!(
        !super::altcha::redeem(&app, &forged.to_payload())
            .await
            .unwrap()
    );

    app.db
        .query("INSERT INTO altcha_used (challenge, expires_at) VALUES ('old', 1)")
        .run()
        .await
        .unwrap();
    super::retention::run(&app).await.unwrap();
    let (left,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM altcha_used")
        .one()
        .await
        .unwrap();
    assert_eq!(left, 1, "only the unexpired challenge stays");
}

/// A raffle with three entrants, frozen and finished without draws.
async fn finished_raffle(app: &AppState) -> RaffleRow {
    let raffle = setup_open(app, &[("Prize", 1)]).await;
    for name in ["Anna", "Bence", "Csilla"] {
        service::add_manual(app, OWNER, &raffle.id, name, None)
            .await
            .unwrap();
    }
    service::start_draw(app, OWNER, &raffle.id).await.unwrap();
    service::command(app, OWNER, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state(), RaffleState::Finished);
    r
}

#[tokio::test]
async fn ending_live_updates() {
    let live = Arc::new(Recording::default());
    let app = app_with_live(Config::for_tests(), live.clone()).await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    assert!(
        !live.notified().is_empty(),
        "lifecycle changes nudge the channel"
    );
    let err = service::end_live(&app, OWNER, &raffle.id)
        .await
        .unwrap_err();
    assert!(matches!(err, service::AppError::Conflict(_)), "{err}");
    assert!(live.ended().is_empty());

    let raffle = finished_raffle(&app).await;
    assert!(raffle.live_ended_at.is_none());
    let view = super::view::build(&app, &raffle.id).await.unwrap().unwrap();
    assert_eq!(view.live, LiveTransport::Partyline { base_url: None });
    assert!(!view.live_ended);

    service::end_live(&app, OWNER, &raffle.id).await.unwrap();
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    let ended = r.live_ended_at.expect("live_ended_at is set");
    assert_eq!(live.ended(), std::slice::from_ref(&raffle.id));
    let view = super::view::build(&app, &raffle.id).await.unwrap().unwrap();
    assert_eq!(view.live, LiveTransport::poll(), "ended: plain polling");
    assert!(view.live_ended);

    // Again: the channel is destroyed again, the time stays.
    service::end_live(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(live.ended().len(), 2);
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.live_ended_at, Some(ended));
    // Other organizers can't.
    assert!(matches!(
        service::end_live(&app, "someone_else", &raffle.id).await,
        Err(service::AppError::NotFound)
    ));
}

/// Deleting and purging a raffle destroy its channel; admin edits that phones
/// see nudge it, joins don't.
#[tokio::test]
async fn writes_nudge_and_destroy_the_channel() {
    let live = Arc::new(Recording::default());
    let app = app_with_live(Config::for_tests(), live.clone()).await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let nudges = || live.notified().len();

    let before = nudges();
    join(&router, &raffle.join_code, "Anna", "anna@example.com").await;
    service::add_manual(&app, OWNER, &raffle.id, "Bence", None)
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
        OWNER,
        &raffle.id,
        &anna,
        crate::model::EntrantPatch {
            hidden: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(nudges(), before + 1, "hiding a name nudges");
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(nudges(), before + 2, "the freeze nudges");

    service::command(&app, OWNER, &raffle.id, Decision::Finish, "fin")
        .await
        .unwrap();
    service::purge(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(live.ended(), std::slice::from_ref(&raffle.id));
    let r = raffles::get(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state(), RaffleState::Purged);
    assert!(r.live_ended_at.is_some(), "a purge ends live updates");
    let (logged,): (i64,) = app
        .db
        .query("SELECT COUNT(*) FROM erasures WHERE raffle_id = ?")
        .bind(&raffle.id)
        .one()
        .await
        .unwrap();
    assert_eq!(logged, 2, "the purge logs its erasures");

    let other = service::create(&app, OWNER, "Other").await.unwrap();
    service::delete(&app, OWNER, &other).await.unwrap();
    assert_eq!(live.ended(), [raffle.id.clone(), other]);
}

/// The retention run ends the live channel of raffles finished over a day
/// ago, whether or not erasure is on, logs every erasure without personal
/// data, and records that it ran.
#[tokio::test]
async fn retention_ends_channels_and_logs_erasures() {
    let live = Arc::new(Recording::default());
    let app = app_with_live(Config::for_tests(), live.clone()).await;
    let old = finished_raffle(&app).await;
    let recent = finished_raffle(&app).await;
    let now = super::ids::now();
    app.db
        .query("UPDATE raffles SET finished_at = ? WHERE id = ?")
        .bind(now - super::retention::CHANNEL_GRACE_SECS - 60)
        .bind(&old.id)
        .run()
        .await
        .unwrap();

    // Erasure is off: only the channel goes.
    let report = super::retention::run(&app).await.unwrap();
    assert_eq!(report.erased, 0);
    assert_eq!(report.channels_ended, 1);
    assert_eq!(live.ended(), std::slice::from_ref(&old.id));
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
    let (ok_at,): (String,) = app
        .db
        .query("SELECT value FROM meta WHERE key = 'last_retention_ok_at'")
        .one()
        .await
        .unwrap();
    assert!(ok_at.parse::<i64>().unwrap() >= now);
    assert_eq!(
        super::retention::run(&app).await.unwrap().channels_ended,
        0,
        "once"
    );

    // Erasure on: the old raffle's non-winners are due.
    let mut cfg = Config::for_tests();
    cfg.retention_enabled = true;
    let app = AppState::from_parts(
        cfg,
        app.db.clone(),
        app.engine.clone(),
        live.clone(),
        Arc::new(Limiter::default()),
    )
    .unwrap();
    app.db
        .query("UPDATE raffles SET purge_nonwinners_at = 0 WHERE id = ?")
        .bind(&old.id)
        .run()
        .await
        .unwrap();
    let report = super::retention::run(&app).await.unwrap();
    assert_eq!(report.erased, 3);
    let entrants = entrants::list(&app.db, &old.id).await.unwrap();
    assert!(
        entrants
            .iter()
            .all(|e| e.purged && e.display_name.starts_with('#'))
    );
    assert!(
        entrants::list(&app.db, &recent.id)
            .await
            .unwrap()
            .iter()
            .all(|e| !e.purged)
    );
    // Not yet purged: the winners' date hasn't passed.
    assert_eq!(
        raffles::get(&app.db, &old.id)
            .await
            .unwrap()
            .unwrap()
            .state(),
        RaffleState::Finished
    );
    let log: Vec<serde_json::Value> = app.db.query("SELECT * FROM erasures").all().await.unwrap();
    assert_eq!(log.len(), 3);
    for row in &log {
        let keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut keys = keys;
        keys.sort();
        assert_eq!(
            keys,
            ["at", "entrant_id", "id", "raffle_id"],
            "no personal data"
        );
        assert_eq!(row["raffle_id"], old.id.as_str());
    }
}
