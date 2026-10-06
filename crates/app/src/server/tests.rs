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
    let report = rafi_core::verify::verify(&data.commit_doc, &data.list_hash, &data.events);
    assert!(report.ok(), "{report:#?}");
    assert_eq!(report.draws.len(), units.len() + 1);

    // CSV export lists the winners (and the not-here entrant).
    let csv = service::export_csv(&app, OWNER, &raffle.id).await.unwrap();
    assert_eq!(csv.lines().count(), 1 + units.len() + 1);

    // Retention: once due, personal data is erased but the draw stays verifiable.
    sqlx::query("UPDATE raffles SET purge_nonwinners_at = 0, purge_winners_at = 0 WHERE id = ?")
        .bind(&raffle.id)
        .execute(&app.db)
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
    assert!(rafi_core::verify::verify(&data.commit_doc, &data.list_hash, &data.events).ok());
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
    assert!(rafi_core::verify::verify(&data.commit_doc, &data.list_hash, &data.events).ok());
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
