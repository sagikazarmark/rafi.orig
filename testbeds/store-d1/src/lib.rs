//! A Worker that runs rafi-store's conformance suite
//! (`rafi_store::conformance`) against D1.
//!
//! ```shell
//! cd testbeds/store-d1
//! wrangler d1 migrations apply rafi-store-d1-check --local
//! wrangler dev --local   # then: curl http://localhost:8787/
//! ```
//!
//! The response maps every scenario to `"ok"`, `"skipped: <reason>"` (a
//! capability D1 lacks) or what went wrong, and is a 500 if any failed.

#![cfg(target_arch = "wasm32")]

use rafi_store::{
    Db,
    conformance::{self, Capabilities, Harness, Verdict},
};
use worker::{Context, Env, Request, Response, Result, event};

/// Local D1 has one database, so each scenario starts by emptying it. Every
/// request runs the whole suite on it again (`workers-check.sh`'s readiness
/// probe is one), so nothing may outlive a scenario.
struct D1(Db);

impl Harness for D1 {
    fn name(&self) -> &str {
        "d1"
    }

    /// D1 runs one statement or batch at a time.
    fn capabilities(&self) -> Capabilities {
        Capabilities::NONE
    }

    /// Empties every table the migrations made, read from the schema, so a
    /// new table can't be forgotten. Raffles go first: their events may only
    /// be deleted once the raffle is gone.
    async fn fresh(&self) -> std::result::Result<Db, String> {
        let tables: Vec<(String,)> = self
            .0
            .query(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table'
                   AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
                   AND name NOT LIKE '\\_cf\\_%' ESCAPE '\\'
                   AND name <> 'd1_migrations'
                 ORDER BY name <> 'raffles', name",
            )
            .all()
            .await
            .map_err(|e| format!("listing the tables: {e}"))?;
        for (table,) in tables {
            self.0
                .query(format!("DELETE FROM \"{table}\""))
                .run()
                .await
                .map_err(|e| format!("emptying {table}: {e}"))?;
        }
        Ok(self.0.clone())
    }
}

#[event(fetch)]
async fn fetch(_req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let verdicts = conformance::run_all(&D1(Db::d1(env.d1("DB")?))).await;
    let failed = verdicts
        .iter()
        .any(|(_, v)| matches!(v, Verdict::Failed(_)));
    let report: serde_json::Map<String, serde_json::Value> = verdicts
        .into_iter()
        .map(|(name, verdict)| {
            let outcome = match verdict {
                Verdict::Passed => "ok".to_owned(),
                Verdict::Skipped(reason) => format!("skipped: {reason}"),
                Verdict::Failed(e) => e,
            };
            (name.to_owned(), outcome.into())
        })
        .collect();
    Ok(Response::from_json(&report)?.with_status(if failed { 500 } else { 200 }))
}
