//! Integration tests: in-memory SQLite, local draw engine, mock drand.

use std::time::Duration;

use dioxus::server::axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use rafi_core::{
    raffle::RaffleState,
    state::{Outcome, UnitStatus},
};
use tower::ServiceExt;

use super::{AppState, altcha::Altcha, config::Config, db, engine::Command, routes, service};
use crate::model::{ItemInput, Lifecycle};

const OWNER: &str = "user_test";

async fn app() -> AppState {
    AppState::new(Config::for_tests()).await.expect("app state")
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
    Altcha::encode(&Altcha::solve(&challenge).expect("solvable"))
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

/// Joins through the attendee page; returns the entrant cookie (if set) and the response.
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

async fn setup_open(app: &AppState, items: &[(&str, u32)]) -> db::RaffleRow {
    let id = service::create(app, OWNER, "Cloud Native Budapest")
        .await
        .unwrap();
    for (i, (name, qty)) in items.iter().enumerate() {
        service::upsert_item(
            app,
            OWNER,
            &id,
            ItemInput {
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
    db::raffle(&app.db, &id).await.unwrap().unwrap()
}

async fn wait_for<F: Fn(&db::RaffleRow) -> bool>(app: &AppState, id: &str, f: F) -> db::RaffleRow {
    for _ in 0..200 {
        let r = db::raffle(&app.db, id).await.unwrap().unwrap();
        if f(&r) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("condition not met in time");
}

/// [`wait_for`] for slow conditions (stalls take a round period plus the timeout).
async fn wait_long<F: Fn(&db::RaffleRow) -> bool>(app: &AppState, id: &str, f: F) -> db::RaffleRow {
    for _ in 0..1000 {
        let r = db::raffle(&app.db, id).await.unwrap().unwrap();
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
    cfg.drand = rafi_drand::DrandConfig {
        relays: vec!["http://127.0.0.1:9".into()],
        mock: false,
        lead_secs: 3,
        timeout_secs: 1,
    };
    AppState::new(cfg).await.expect("app state")
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

    // Attendees join; the cookie brings them back to their entry.
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
    let me = db::entrants(&app.db, &raffle.id)
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
    assert_eq!(db::entrants(&app.db, &raffle.id).await.unwrap().len(), 6);

    // Freeze; joining is closed afterwards.
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.draw_state.is_some()).await;
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
        service::upsert_item(
            &app,
            OWNER,
            &raffle.id,
            ItemInput {
                name: "x".into(),
                qty: 1,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );

    // Draw every unit; the first winner is not here and the unit is redrawn.
    let state = r.draw_state().unwrap();
    let units: Vec<String> = state.units.iter().map(|u| u.unit_id.clone()).collect();
    let mut winners = Vec::new();
    for (i, unit) in units.iter().enumerate() {
        service::command(
            &app,
            OWNER,
            &raffle.id,
            Command::Draw {
                unit_id: unit.clone(),
                offline: false,
            },
            &format!("c{i}"),
        )
        .await
        .unwrap();
        let r = wait_for(&app, &raffle.id, |r| {
            r.draw_state()
                .is_some_and(|s| s.unit(unit).unwrap().status == UnitStatus::Drawn)
        })
        .await;
        if i == 0 {
            let first = r
                .draw_state()
                .unwrap()
                .unit(unit)
                .unwrap()
                .entrant_id
                .clone()
                .unwrap();
            service::command(
                &app,
                OWNER,
                &raffle.id,
                Command::Resolve {
                    unit_id: unit.clone(),
                    outcome: Outcome::NotHere,
                },
                "nh",
            )
            .await
            .unwrap();
            service::command(
                &app,
                OWNER,
                &raffle.id,
                Command::Draw {
                    unit_id: unit.clone(),
                    offline: false,
                },
                "redraw",
            )
            .await
            .unwrap();
            let r = wait_for(&app, &raffle.id, |r| {
                r.draw_state()
                    .is_some_and(|s| s.unit(unit).unwrap().status == UnitStatus::Drawn)
            })
            .await;
            assert_ne!(
                r.draw_state()
                    .unwrap()
                    .unit(unit)
                    .unwrap()
                    .entrant_id
                    .clone()
                    .unwrap(),
                first
            );
        }
        let r = db::raffle(&app.db, &raffle.id).await.unwrap().unwrap();
        winners.push(
            r.draw_state()
                .unwrap()
                .unit(unit)
                .unwrap()
                .entrant_id
                .clone()
                .unwrap(),
        );
        service::command(
            &app,
            OWNER,
            &raffle.id,
            Command::Resolve {
                unit_id: unit.clone(),
                outcome: Outcome::Claimed,
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

    // Drawing a claimed unit again is rejected.
    assert!(
        service::command(
            &app,
            OWNER,
            &raffle.id,
            Command::Draw {
                unit_id: units[0].clone(),
                offline: false
            },
            "x"
        )
        .await
        .is_err()
    );

    service::command(&app, OWNER, &raffle.id, Command::Finish, "fin")
        .await
        .unwrap();
    let r = db::raffle(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(r.state(), RaffleState::Finished);
    assert!(r.purge_nonwinners_at.is_some());

    // The public log verifies independently.
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let report = rafi_core::verify::verify(
        &data.commit_doc,
        &data.list_hash,
        &data.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), units.len() + 1);

    // CSV export lists the winners (and the not-here entrant).
    let csv = service::export_csv(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(csv.lines().count(), 1 + units.len() + 1);

    // Retention: once due, personal data is erased but the draw stays verifiable.
    app.db
        .query("UPDATE raffles SET purge_nonwinners_at = 0, purge_winners_at = 0 WHERE id = ?")
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    let erased = super::retention::run(&app).await.unwrap();
    assert_eq!(erased, 6);
    let entrants = db::entrants(&app.db, &raffle.id).await.unwrap();
    assert!(
        entrants
            .iter()
            .all(|e| e.purged && e.email_raw.is_none() && e.token_hash.is_none())
    );
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(
        rafi_core::verify::verify(
            &data.commit_doc,
            &data.list_hash,
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
    wait_for(&app, &raffle.id, |r| r.draw_state.is_some()).await;
    let state = db::raffle(&app.db, &raffle.id)
        .await
        .unwrap()
        .unwrap()
        .draw_state()
        .unwrap();
    service::command(
        &app,
        OWNER,
        &raffle.id,
        Command::Draw {
            unit_id: state.units[0].unit_id.clone(),
            offline: false,
        },
        "c",
    )
    .await
    .unwrap();
    wait_for(&app, &raffle.id, |r| {
        r.draw_state()
            .is_some_and(|s| s.units[0].status == UnitStatus::Drawn)
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
    let claim = db::entrants(&app.db, &raffle.id).await.unwrap()[0]
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

#[tokio::test]
async fn sse_sends_current_view_first() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    let resp = router
        .clone()
        .oneshot(
            Request::get(format!("/api/r/{}/events", raffle.slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get(header::CONTENT_TYPE).unwrap(),
        "text/event-stream"
    );
    let mut body = resp.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(2), body.frame())
        .await
        .expect("first event")
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(frame.data_ref().unwrap()).into_owned();
    assert!(text.starts_with("event: view"), "{text}");
    assert!(text.contains(&raffle.join_code));
}

#[tokio::test]
async fn drafts_are_private() {
    let app = app().await;
    let router = routes(app.clone());
    let id = service::create(&app, OWNER, "Secret").await.unwrap();
    let raffle = db::raffle(&app.db, &id).await.unwrap().unwrap();
    let (status, _, _) = send(
        &router,
        Request::get(format!("/j/{}", raffle.join_code))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = send(
        &router,
        Request::get(format!("/api/r/{}/events", raffle.slug))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
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
    assert!(db::entrants(&app.db, &raffle.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn offline_draw_and_engine_switch() {
    let app = app().await;
    let router = routes(app.clone());
    let raffle = setup_open(&app, &[("Prize", 2)]).await;
    for i in 0..3 {
        join(
            &router,
            &raffle.join_code,
            &format!("P{i}"),
            &format!("p{i}@example.com"),
        )
        .await;
    }
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.draw_state.is_some()).await;
    let unit = r.draw_state().unwrap().units[0].unit_id.clone();
    service::command(
        &app,
        OWNER,
        &raffle.id,
        Command::Draw {
            unit_id: unit.clone(),
            offline: true,
        },
        "o",
    )
    .await
    .unwrap();
    let r = db::raffle(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(
        r.draw_state().unwrap().unit(&unit).unwrap().status,
        UnitStatus::Drawn
    );
    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    assert!(
        rafi_core::verify::verify(
            &data.commit_doc,
            &data.list_hash,
            &data.events,
            rafi_drand::resolve_chain
        )
        .ok()
    );
    // Switching an already-local raffle is a no-op.
    service::switch_to_local(&app, OWNER, &raffle.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn admin_requires_clerk_session() {
    use dioxus_clerk::testing::TestClerk;
    let app = app().await;
    let clerk = TestClerk::new().unwrap();
    // The Dioxus router serves the client bundle from `public/` next to the binary.
    let public = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("public");
    std::fs::create_dir_all(&public).unwrap();
    std::fs::write(
        public.join("index.html"),
        "<!doctype html><html><head></head><body><div id=\"main\"></div></body></html>",
    )
    .unwrap();
    let router = super::build_router(app.clone(), Some(clerk.layer().unwrap()));

    let (status, _, _) = send(
        &router,
        Request::post("/api/admin/whoami")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert!(
        !status.is_success(),
        "anonymous request must fail, got {status}"
    );

    let (status, _, body) = send(
        &router,
        Request::post("/api/admin/whoami")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::COOKIE, clerk.cookie("user_2abc").unwrap())
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert!(status.is_success(), "{status}: {body}");
    assert!(body.contains("user_2abc"), "{body}");
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
    service::upsert_item(
        &app,
        OWNER,
        &id,
        ItemInput {
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
        db::raffle(&app.db, &id).await.unwrap().unwrap().state(),
        RaffleState::Open
    );
    assert!(
        service::add_fakes(&app, OWNER, &id, 3).await.is_err(),
        "fakes need test mode"
    );
    let dup = service::duplicate(&app, OWNER, &id).await.unwrap();
    assert_eq!(db::items(&app.db, &dup).await.unwrap().len(), 1);
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
    let r = wait_for(&app, &raffle.id, |r| r.draw_state.is_some()).await;
    let doc: rafi_core::commit::CommitDoc =
        serde_json::from_str(r.commit_doc.as_deref().expect("commit document")).unwrap();
    assert_eq!(doc.entrant_ids.len(), 3);
    assert_eq!(doc.units.len(), 2);
    assert_eq!(
        r.list_hash.as_deref(),
        Some(
            hex::encode(rafi_core::commit::list_hash(
                r.commit_doc.as_deref().unwrap()
            ))
            .as_str()
        )
    );
    assert!(r.frozen_at.is_some());
    assert_eq!(r.draw_state().unwrap().list_hash, r.list_hash.unwrap());

    // Starting again re-sends the same commitment.
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let again = db::raffle(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(again.commit_doc, r.commit_doc);
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
    let entrants = db::entrants(&app.db, &raffle.id).await.unwrap();
    assert_eq!(entrants.len(), service::MAX_FAKE_ENTRANTS as usize);
    assert!(entrants.iter().all(|e| e.source == "fake" && !e.hidden));
    let ids: std::collections::HashSet<_> = entrants.iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), entrants.len());
}

#[tokio::test]
async fn the_restate_watcher_only_watches_when_asked() {
    let app = app().await;
    assert!(
        app.watch.is_idle(),
        "nothing is queried until a Restate command is sent"
    );

    // A local raffle (or a missing one) is not Restate's to write: dropped at once.
    let id = service::create(&app, "owner", "Local").await.unwrap();
    app.watch.wake(&id);
    app.watch.wake("missing");
    let mut watcher = super::watch::Watcher::default();
    assert!(watcher.poll(&app).await.unwrap().is_empty());
    assert!(app.watch.is_idle());
}

/// A draw whose round never arrives in time stalls on that round: retrying
/// keeps the round, pool-changing resolutions wait, and the only exits
/// (offline, abandon) are logged for the verifier.
#[tokio::test]
async fn stalled_draw_keeps_its_round() {
    use rafi_core::{state::Event, verify::Replaced};

    let app = unreachable_drand_app().await;
    let raffle = setup_open(&app, &[("Prize", 3)]).await;
    for i in 0..4 {
        service::add_manual(&app, OWNER, &raffle.id, &format!("Person {i}"), None)
            .await
            .unwrap();
    }
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.draw_state.is_some()).await;
    let units: Vec<String> = r
        .draw_state()
        .unwrap()
        .units
        .iter()
        .map(|u| u.unit_id.clone())
        .collect();
    async fn draw_cmd(
        app: &AppState,
        raffle_id: &str,
        unit: &str,
        offline: bool,
        id: &str,
    ) -> super::service::Result<()> {
        service::command(
            app,
            OWNER,
            raffle_id,
            Command::Draw {
                unit_id: unit.to_owned(),
                offline,
            },
            id,
        )
        .await
    }
    let draw = |unit: &str, offline: bool, id: &str| {
        let (unit, id) = (unit.to_owned(), id.to_owned());
        let (app, raffle_id) = (app.clone(), raffle.id.clone());
        async move { draw_cmd(&app, &raffle_id, &unit, offline, &id).await }
    };
    let stalls = |r: &db::RaffleRow| {
        r.draw_state()
            .map(|s| {
                s.events
                    .iter()
                    .filter(|e| matches!(e, Event::Stall { .. }))
                    .count()
            })
            .unwrap_or_default()
    };

    draw(&units[0], false, "d1").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 1).await;
    let pending = r.draw_state().unwrap().pending.expect("still pending");

    // Another unit cannot start; this one retries the same round.
    assert!(draw(&units[1], false, "d2").await.is_err());
    draw(&units[0], false, "d3").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 2).await;
    let s = r.draw_state().unwrap();
    assert_eq!(s.pending.as_ref(), Some(&pending), "same draw, same round");
    let announces = s
        .events
        .iter()
        .filter(|e| matches!(e, Event::Announce { .. }))
        .count();
    assert_eq!(announces, 1);

    // The organizer abandons it: logged, and the unit can be drawn again.
    service::command(
        &app,
        OWNER,
        &raffle.id,
        Command::Abandon {
            unit_id: units[0].clone(),
        },
        "a1",
    )
    .await
    .unwrap();
    // Then completes the next draw offline instead of waiting for its round.
    draw(&units[0], false, "d4").await.unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 3).await;
    let next = r.draw_state().unwrap().pending.unwrap();
    assert_eq!(next.k, pending.k, "the abandoned draw's number is reused");
    assert!(
        next.round >= pending.round,
        "a new draw commits to a current round"
    );
    draw(&units[0], true, "o1").await.unwrap();
    let r = db::raffle(&app.db, &raffle.id).await.unwrap().unwrap();
    assert_eq!(
        r.draw_state().unwrap().unit(&units[0]).unwrap().status,
        UnitStatus::Drawn
    );

    // While the next draw is pending, "not here" waits; "handed over" doesn't.
    draw(&units[1], false, "d5").await.unwrap();
    let not_here = service::command(
        &app,
        OWNER,
        &raffle.id,
        Command::Resolve {
            unit_id: units[0].clone(),
            outcome: Outcome::NotHere,
        },
        "nh",
    )
    .await;
    assert!(not_here.is_err());
    service::command(
        &app,
        OWNER,
        &raffle.id,
        Command::Resolve {
            unit_id: units[0].clone(),
            outcome: Outcome::Claimed,
        },
        "cl",
    )
    .await
    .unwrap();

    let data = service::verify_data(&app, &raffle.slug).await.unwrap();
    let report = rafi_core::verify::verify(
        &data.commit_doc,
        &data.list_hash,
        &data.events,
        rafi_drand::resolve_chain,
    );
    assert!(report.ok(), "{report:#?}");
    let replaced: Vec<_> = report
        .replaced
        .iter()
        .map(|c| (c.round, c.replaced))
        .collect();
    assert_eq!(
        replaced,
        [
            (pending.round, Replaced::Abandoned),
            (next.round, Replaced::Offline)
        ]
    );
    assert_eq!(report.replaced[1].k, pending.k);
    assert!(rafi_core::verify::verify_public(&data.commit_doc, &data.list_hash, &data.events).ok());
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
    let public = rafi_core::verify::verify_public(&data.commit_doc, &data.list_hash, &data.events);
    assert!(public.chain_unknown() && !public.ok());
}

/// Switching a Restate raffle with a stalled draw to the local engine keeps
/// the draw on its round, and the local engine fetches that round.
#[tokio::test]
async fn switching_engines_keeps_a_stalled_draw() {
    use rafi_core::state::Event;

    let app = unreachable_drand_app().await;
    let raffle = setup_open(&app, &[("Prize", 1)]).await;
    service::add_manual(&app, OWNER, &raffle.id, "Person", None)
        .await
        .unwrap();
    service::start_draw(&app, OWNER, &raffle.id).await.unwrap();
    let r = wait_for(&app, &raffle.id, |r| r.draw_state.is_some()).await;
    let unit = r.draw_state().unwrap().units[0].unit_id.clone();
    service::command(
        &app,
        OWNER,
        &raffle.id,
        Command::Draw {
            unit_id: unit,
            offline: false,
        },
        "d1",
    )
    .await
    .unwrap();
    let stalls = |r: &db::RaffleRow| {
        r.draw_state()
            .map(|s| {
                s.events
                    .iter()
                    .filter(|e| matches!(e, Event::Stall { .. }))
                    .count()
            })
            .unwrap_or_default()
    };
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 1).await;
    let pending = r.draw_state().unwrap().pending.unwrap();

    // Pretend Restate was drawing it; a raffle on another chain is refused.
    let mut other = r.draw_state().unwrap();
    other.drand_chain = "00".repeat(32);
    app.db
        .query("UPDATE raffles SET engine = 'restate', draw_state = ? WHERE id = ?")
        .bind(serde_json::to_string(&other).unwrap())
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();
    assert!(
        service::switch_to_local(&app, OWNER, &raffle.id)
            .await
            .is_err()
    );
    app.db
        .query("UPDATE raffles SET draw_state = ? WHERE id = ?")
        .bind(r.draw_state.clone())
        .bind(&raffle.id)
        .run()
        .await
        .unwrap();

    service::switch_to_local(&app, OWNER, &raffle.id)
        .await
        .unwrap();
    let r = wait_long(&app, &raffle.id, |r| stalls(r) >= 2).await;
    assert_eq!(r.engine, "local");
    assert_eq!(
        r.draw_state().unwrap().pending,
        Some(pending),
        "same draw, same round"
    );
}
