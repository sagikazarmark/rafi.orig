//! A Worker that hosts `RaffleChannel` and lets `scenario.mjs` change a raffle
//! the way the app and the drawing engine would.
//!
//! ```shell
//! cd crates/cloudflare-check
//! wrangler d1 migrations apply rafi-cloudflare-check --local
//! wrangler dev --local --port 8790 &
//! node scenario.mjs http://127.0.0.1:8790
//! ```

#![cfg(target_arch = "wasm32")]

use rafi_cloudflare::DurableObjects;
pub use rafi_cloudflare::RaffleChannel;
use rafi_drawing::{BeaconSource, Drawing, Event, ItemStatus, Manifest, ManifestItem};
use rafi_platform::Notifier;
use rafi_store::{Db, PurgeAfter, live::Snapshot};
use worker::{Context, Env, Method, Request, Response, Result, event};

fn err(e: impl std::fmt::Display) -> worker::Error {
    worker::Error::RustError(e.to_string())
}

/// Entrant IDs are global primary keys, so each run's raffle gets its own.
fn entrant(raffle: &str, name: &str) -> String {
    format!("{raffle}-{name}")
}

async fn drawing(db: &Db, raffle: &str) -> Result<Drawing> {
    rafi_store::raffles::get(db, raffle)
        .await
        .map_err(err)?
        .map(|r| r.drawing())
        .transpose()
        .map_err(err)?
        .flatten()
        .ok_or_else(|| err("the raffle has no drawing"))
}

async fn reset(db: &Db, raffle: &str) -> Result<()> {
    db.batch(vec![
        db.query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, engine, created_at, updated_at)
             VALUES (?1, 'o', ?1, ?1, 't', 'open', 'restate', 0, 0)",
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
    let state = Drawing::open(&manifest(raffle, BeaconSource::QUICKNET)).map_err(err)?;
    db.query("UPDATE raffles SET state = 'frozen', manifest_hash = ?, drawing = ?, drawing_version = ? WHERE id = ?")
        .bind(state.manifest_hash().to_string())
        .bind(serde_json::to_string(&state).map_err(err)?)
        .bind(state.version())
        .bind(raffle)
        .run()
        .await
        .map_err(err)?;
    Ok(())
}

/// The app's half of a Restate freeze: the commit document (on the mock drand
/// chain the Restate Worker uses in development) and the frozen raffle row.
/// Returns the manifest to hand to `Drawing/{id}/open`.
async fn prepare(db: &Db, raffle: &str) -> Result<String> {
    let chain = rafi_drand::DrandClient::new(rafi_drand::DrandConfig {
        mock: true,
        ..Default::default()
    })
    .map_err(err)?
    .chain_hash();
    let doc = manifest(raffle, BeaconSource::Drand(chain));
    let json = doc.canonical_json();
    db.query("UPDATE raffles SET state = 'frozen', manifest = ?, manifest_hash = ? WHERE id = ?")
        .bind(json.as_str())
        .bind(doc.hash().to_string())
        .bind(raffle)
        .run()
        .await
        .map_err(err)?;
    Ok(json)
}

const STRINGS: &[(&str, &str)] = &[
    ("you_won", "You won!"),
    ("go_to_organizer", "Go to the organizer"),
    ("thanks_won", "Thanks, enjoy!"),
    ("thanks", "Thanks for joining"),
    ("drawing", "Drawing"),
    ("not_this_time", "Not this time"),
    ("draw_starting", "The draw is starting"),
    ("prizes_left", "prizes left"),
    ("waiting", "Waiting for the draw"),
    ("locking", "Locking"),
    ("drand_round", "drand round"),
];

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
    use rafi_live::{Card, CardStrings};

    let snap = Snapshot::read(db, raffle)
        .await
        .map_err(err)?
        .ok_or_else(|| err("no such raffle"))?;
    let view = snap.view();
    let strings = CardStrings::from_lookup(|k| {
        STRINGS
            .iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| (*v).to_owned())
            .unwrap_or_default()
    });
    let me = me.map(|m| entrant(raffle, m));
    let now_ms = js_sys::Date::now() as u64;
    let card = dioxus::ssr::render_element(rsx! {
        Card {
            view: view.clone(),
            me: me.clone(),
            claim_code: me.as_ref().map(|_| "K2P9".to_owned()),
            page_state: snap.state,
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
        "live": DurableObjects::new(env.clone()).transport(raffle),
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
<script>
window.rafiFallback=function(){{if(window.__rafiFallback)return;window.__rafiFallback=1;var s=document.createElement("script");s.src="/participant/fallback.js";document.body.appendChild(s)}};
setTimeout(function(){{if(!window.__rafiReady)window.rafiFallback()}},8000);
if(typeof WebAssembly!=="object"){{window.rafiFallback()}}else{{import("/participant/rafi_participant.js").then(function(m){{return m.default()}}).catch(function(e){{console.error(e);window.rafiFallback()}})}}
</script></body></html>"#
    );
    Response::from_html(html)
}

fn altcha_challenge() -> rafi_live::altcha::Challenge {
    let random = || (js_sys::Math::random() * u32::MAX as f64) as u64;
    let mut salt = [0u8; 12];
    for b in &mut salt {
        *b = random() as u8;
    }
    let now = (js_sys::Date::now() / 1000.0) as u64;
    rafi_live::altcha::issue(b"check-key", 30_000, now, random() % 30_000, salt)
}

/// Appends `event` to the stored drawing, the way the engine's projection
/// writes it.
async fn record(db: &Db, raffle: &str, event: Event) -> Result<()> {
    let mut events = drawing(db, raffle).await?.events().to_vec();
    events.push(event);
    let state =
        Drawing::restore(&manifest(raffle, BeaconSource::QUICKNET), &events).map_err(err)?;
    rafi_store::save_drawing(db, &state, PurgeAfter::days(30, 90))
        .await
        .map_err(err)?;
    Ok(())
}

/// A completed draw of the next prize for `who`.
async fn draw(db: &Db, raffle: &str, who: &str) -> Result<()> {
    let state = drawing(db, raffle).await?;
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
    let Some(raffle) = param("r") else {
        return Response::error("missing ?r=<raffle id>", 400);
    };
    let raffle = raffle.as_str();
    if path == "/api/altcha" {
        return Response::from_json(&altcha_challenge());
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
        let snap = Snapshot::read(&db, raffle).await.map_err(err)?;
        return Response::from_json(&snap.map(|s| s.view()));
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
        "/prepare" => return Response::ok(prepare(&db, raffle).await?),
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
