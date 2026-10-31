//! A Worker that hosts `RaffleRoom` and lets `scenario.mjs` change a raffle
//! the way the app and the raffle engine would.
//!
//! ```shell
//! cd testbeds/cloudflare
//! wrangler d1 migrations apply rafi-cloudflare-check --local
//! wrangler dev --local --port 8790 &
//! node scenario.mjs http://127.0.0.1:8790
//! ```

#![cfg(target_arch = "wasm32")]

use rafi_cloudflare::DurableObjects;
pub use rafi_cloudflare::RaffleRoom;
use rafi_live_server::raffle::view as participant_view;
use rafi_raffle::{BeaconSource, Event, ItemStatus, Manifest, ManifestItem, Raffle};
use rafi_store::{Db, live};
use worker::{Context, Env, Method, Request, Response, Result, event};

fn err(e: impl std::fmt::Display) -> worker::Error {
    worker::Error::RustError(e.to_string())
}

/// Entrant IDs are global primary keys, so each run's raffle gets its own.
fn entrant(raffle: &str, name: &str) -> String {
    format!("{raffle}-{name}")
}

async fn drawn(db: &Db, raffle: &str) -> Result<Raffle> {
    let row = rafi_store::raffles::get(db, raffle)
        .await
        .map_err(err)?
        .ok_or_else(|| err("no such raffle"))?;
    rafi_store::raffles::drawn(db, &row)
        .await
        .map_err(err)?
        .ok_or_else(|| err("the raffle is not opened"))
}

async fn reset(db: &Db, raffle: &str) -> Result<()> {
    db.batch(vec![
        db.query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, event_date, event_ends_at,
                                  retention_days, created_at, updated_at)
             VALUES (?1, 'o', ?1, ?1, 't', 'open', '2100-01-01', 4102444800, 7, 0, 0)",
        )
        .bind(raffle),
        db.query(
            "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at)
             VALUES (?, ?, 'Anna', 'K2P9', 0), (?, ?, 'Bence', 'H7TX', 0)",
        )
        .bind(entrant(raffle, "anna"))
        .bind(raffle)
        .bind(entrant(raffle, "bence"))
        .bind(raffle),
        db.query("INSERT INTO prizes (id, raffle_id, name, qty) VALUES (?1 || '-book', ?1, 'Book', 1), (?1 || '-shirt', ?1, 'Shirt', 1)")
            .bind(raffle),
    ])
    .await
    .map_err(err)?;
    Ok(())
}

/// The raffle's manifest: Anna and Bence, a book and a shirt.
fn manifest(raffle: &str, beacon: BeaconSource) -> Manifest {
    Manifest::new(
        raffle,
        beacon,
        vec![
            entrant(raffle, "anna").into(),
            entrant(raffle, "bence").into(),
        ],
        vec![
            ManifestItem {
                id: "book-1".into(),
                name: "Book".into(),
            },
            ManifestItem {
                id: "shirt-1".into(),
                name: "Shirt".into(),
            },
        ],
    )
}

async fn freeze(db: &Db, raffle: &str) -> Result<()> {
    let doc = manifest(raffle, BeaconSource::QUICKNET);
    db.query("UPDATE raffles SET state = 'frozen', manifest = ?, manifest_hash = ? WHERE id = ?")
        .bind(doc.canonical_json())
        .bind(doc.hash().to_string())
        .bind(raffle)
        .run()
        .await
        .map_err(err)?;
    let state = Raffle::open(&doc).map_err(err)?;
    save(db, &state).await
}

/// The page's loader, from the participant build, as rafi-server compiles it
/// in (`scripts/workers-check.sh` builds it first, and puts the
/// build's assets and fallback under `site/participant/`).
const LOADER: &str = include_str!("../../../apps/participant/dist/public/index.html");

/// A stand-in for the participant page: the server-rendered card, the embedded
/// config, the inline loader with its fallback, and the WASM. `me` picks the
/// entrant (`anna`, `bence`, or none for a visitor); `nowasm=1` removes
/// `WebAssembly` to exercise the fallback.
async fn page(
    env: &Env,
    db: &Db,
    raffle: &str,
    me: Option<&str>,
    nowasm: bool,
) -> Result<Response> {
    use dioxus::prelude::*;
    use rafi_live_client::{Card, CardStrings};

    let snap = live::snapshot(db, raffle)
        .await
        .map_err(err)?
        .ok_or_else(|| err("no such raffle"))?;
    let view = participant_view(&snap);
    // Its own strings, not the defaults: smoke.mjs checks they reach the card.
    let strings = CardStrings {
        you_won: "You won!".into(),
        go_to_organizer: "Go to the organizer".into(),
        thanks_won: "Thanks, enjoy!".into(),
        thanks: "Thanks for joining".into(),
        drawing: "Drawing".into(),
        not_this_time: "Not this time".into(),
        draw_starting: "The draw is starting".into(),
        prizes_left: "prizes left".into(),
        waiting: "Waiting for the draw".into(),
        locking: "Locking".into(),
        drand_round: "drand round".into(),
    };
    let me = me.map(|m| entrant(raffle, m));
    let now_ms = js_sys::Date::now() as u64;
    let card = dioxus::ssr::render_element(rsx! {
        Card {
            view: view.clone(),
            me: me.clone(),
            claim_code: me.as_ref().map(|_| "K2P9".to_owned()),
            page_state: view.state,
            now_ms,
            strings: strings.clone(),
        }
    });
    let config = serde_json::json!({
        "raffle_id": raffle,
        "entrant_id": me,
        "claim_code": me.as_ref().map(|_| "K2P9"),
        "page_state": snap.state,
        "view": view,
        "strings": strings,
        "participant_url": format!("/participant?r={raffle}"),
        "server_now_ms": now_ms,
        "follow": DurableObjects::new(env.clone()).follow(raffle),
        "screen_on": "Screen stays on",
        "keep_screen_on": "Keep the screen on",
    });
    let config = config.to_string().replace("</", "<\\/");
    let nowasm = if nowasm {
        "<script>delete window.WebAssembly;</script>"
    } else {
        ""
    };
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8">
<script>window.__rafiT0=Date.now();try{{var t=localStorage.getItem("rafi-theme");if(t)document.documentElement.setAttribute("data-theme",t)}}catch(e){{}}</script>
<style>.hidden{{display:none}}</style><title>rafi check</title></head><body>
<header><button id="theme" type="button"><span class="theme-icon-system">🖥</span><span class="theme-icon-light hidden">☀️</span><span class="theme-icon-dark hidden">🌙</span></button></header>
<section id="live" aria-live="polite">{card}</section>
<button id="wake" type="button">Keep the screen on</button>
<script type="application/json" id="rafi-config">{config}</script>
{nowasm}
{LOADER}</body></html>"#
    );
    Response::from_html(html)
}

fn pow_challenge() -> altcha::Challenge {
    let now = (js_sys::Date::now() / 1000.0) as i64;
    // As the app issues them (rafi_app::pow), cheap.
    altcha::create_challenge(altcha::CreateChallengeOptions {
        algorithm: "PBKDF2/SHA-256".into(),
        cost: 100,
        key_prefix: "00".into(),
        expires_at: Some(now + 20 * 60),
        hmac_signature_secret: Some("check-key".into()),
        ..Default::default()
    })
    .expect("valid options")
}

/// Appends `event` to the stored raffle, the way the engine's projection
/// writes it.
async fn record(db: &Db, raffle: &str, event: Event) -> Result<()> {
    let mut events = drawn(db, raffle).await?.events().to_vec();
    events.push(event);
    let state = Raffle::restore(&manifest(raffle, BeaconSource::QUICKNET), &events).map_err(err)?;
    save(db, &state).await
}

/// Writes `state` as the engine would; one that isn't written is an error.
async fn save(db: &Db, state: &Raffle) -> Result<()> {
    if rafi_store::save_raffle(db, state).await.map_err(err)? {
        Ok(())
    } else {
        Err(err("the raffle was not saved"))
    }
}

/// A completed draw of the next prize for `who`.
async fn draw(db: &Db, raffle: &str, who: &str) -> Result<()> {
    let state = drawn(db, raffle).await?;
    let Some(item) = state
        .items()
        .iter()
        .find(|u| u.status() == ItemStatus::Available)
    else {
        return Err(err("no prize left"));
    };
    let draw = Event::Drawn {
        k: state.draws().len() as u64 + 1,
        item: item.id.clone(),
        round: 1,
        signature: String::new(),
        randomness: String::new(),
        pass: 1,
        pool_size: 2,
        index: 0,
        entrant: entrant(raffle, who).into(),
    };
    record(db, raffle, draw).await
}

async fn finish(db: &Db, raffle: &str) -> Result<()> {
    record(db, raffle, Event::Finished).await
}

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let db = rafi_cloudflare::database(&env)?;
    let url = req.url()?;
    let path = url.path().to_owned();
    let param = |name: &str| {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    let nudge_after = param("nudge").as_deref() != Some("0");
    if let Some(id) = path.strip_prefix("/partyline/raffle/") {
        return rafi_cloudflare::connect(&env, id, req).await;
    }
    // The participant page's proof-of-work solver, as rafi-server serves it.
    if path == "/participant/pow-worker.js" {
        let mut resp = Response::ok(dioxus_altcha::WORKER_JS)?;
        resp.headers_mut()
            .set("content-type", "text/javascript; charset=utf-8")?;
        return Ok(resp);
    }
    let Some(raffle) = param("r") else {
        return Response::error("missing ?r=<raffle id>", 400);
    };
    let raffle = raffle.as_str();
    if path == "/api/challenge" {
        return Response::from_json(&pow_challenge());
    }
    if path == "/page" {
        let Some(raffle) = param("r") else {
            return Response::error("missing ?r=<raffle id>", 400);
        };
        return page(
            &env,
            &db,
            &raffle,
            param("me").as_deref(),
            param("nowasm").is_some(),
        )
        .await;
    }
    if path == "/participant" {
        let snap = live::snapshot(&db, raffle).await.map_err(err)?;
        return Response::from_json(&snap.as_ref().map(participant_view));
    }
    if req.method() != Method::Post {
        return Response::error("Not Found", 404);
    }
    match path.as_str() {
        "/reset" => reset(&db, raffle).await?,
        "/join" => {
            db.query("INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at) VALUES (?, ?, 'Cora', 'Q4M7', 0)")
                .bind(entrant(raffle, "cora"))
                .bind(raffle)
                .run()
                .await
                .map_err(err)?;
        }
        "/ready" => {
            db.query("UPDATE raffles SET state = 'ready' WHERE id = ?")
                .bind(raffle)
                .run()
                .await
                .map_err(err)?;
        }
        "/open" => {
            db.query("UPDATE raffles SET state = 'open' WHERE id = ?")
                .bind(raffle)
                .run()
                .await
                .map_err(err)?;
        }
        "/close" => {
            db.query("UPDATE raffles SET state = 'closed' WHERE id = ?")
                .bind(raffle)
                .run()
                .await
                .map_err(err)?;
        }
        "/freeze" => freeze(&db, raffle).await?,
        "/draw/anna" => draw(&db, raffle, "anna").await?,
        "/draw/bence" => draw(&db, raffle, "bence").await?,
        "/finish" => finish(&db, raffle).await?,
        "/end" => {
            db.query("UPDATE raffles SET live_ended_at = 1 WHERE id = ?")
                .bind(raffle)
                .run()
                .await
                .map_err(err)?;
            rafi_cloudflare::destroy(&env, raffle).await?;
            return Response::ok("destroyed");
        }
        "/nudge" => {}
        _ => return Response::error("Not Found", 404),
    }
    if nudge_after {
        return Response::ok(rafi_cloudflare::nudge(&env, raffle).await?);
    }
    Response::ok("written")
}
