//! A Worker that runs `rafi_store::testsuite` against D1.
//!
//! ```shell
//! cd crates/store-d1-check
//! wrangler d1 migrations apply rafi-store-check --local
//! wrangler dev --local   # then: curl http://localhost:8787/
//! ```
//!
//! The response lists every scenario and fails with 500 if any failed.

#![cfg(target_arch = "wasm32")]

use rafi_store::Db;
use worker::{Context, Env, Request, Response, Result, event};

/// Local D1 has one database, so each scenario starts by emptying it.
async fn clear(db: &Db) -> Db {
    for table in [
        "audit",
        "entrants",
        "items",
        "raffles",
        "altcha_used",
        "erasures",
        "meta",
    ] {
        db.query(format!("DELETE FROM {table}"))
            .run()
            .await
            .expect("clearing a table");
    }
    db.clone()
}

#[event(fetch)]
async fn fetch(_req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let db = Db::d1(env.d1("DB")?);
    let results = rafi_store::testsuite::run(|| clear(&db)).await;
    let failed = results.iter().any(|(_, r)| r.is_err());
    let report: serde_json::Map<String, serde_json::Value> = results
        .into_iter()
        .map(|(name, r)| (name.to_owned(), r.err().map_or("ok".into(), |e| e.into())))
        .collect();
    Ok(Response::from_json(&report)?.with_status(if failed { 500 } else { 200 }))
}
