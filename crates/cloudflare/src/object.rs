//! The `RaffleChannel` Durable Object: one per raffle, pushing the participant
//! view to phones over a partyline Latest channel.
//!
//! It holds no workflow state. Whenever it is nudged (and every 15 s while
//! sockets are open, to repair lost nudges) it reads the raffle from D1 in one
//! snapshot and publishes the view if it is newer and different. Its own
//! storage keeps only the raffle ID, the highest version seen and the last
//! published view; destroying it deletes all of that.
//!
//! Requests come from this Worker only (see [`crate::gate`]): the raffle ID
//! travels in the `x-rafi-raffle` header.

use std::{cell::Cell, time::Duration};

use partyline_worker::{Hub, HubConfig, close, reject};
use rafi_live::{ParticipantView, RaffleLive};
use rafi_platform::rule::{Decision, decide};
use rafi_store::{Db, live::Snapshot};
use worker::{
    DurableObject, Env, Method, Request, Response, Result, SqlStorageValue, State, durable_object,
    wasm_bindgen,
};

/// The header carrying the raffle ID on requests to the object.
pub(crate) const RAFFLE_HEADER: &str = "x-rafi-raffle";

/// How often the object re-reads the raffle while sockets are open.
const REREAD: Duration = Duration::from_secs(15);

/// The most sockets one raffle accepts; the rest fall back to polling.
const MAX_SOCKETS: usize = 2_000;

/// The largest participant view the hub publishes. It grows with the winners
/// (about 100 bytes each), and prizes aren't capped in number, so partyline's
/// 64 KiB default could refuse a big raffle's view and leave phones on the
/// last one. Under the 2 MB a Durable Object's SQLite row may hold.
const MAX_VIEW_BYTES: usize = 1024 * 1024;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS rafi_channel (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    raffle_id TEXT NOT NULL,
    seen_v INTEGER NOT NULL DEFAULT 0,
    last TEXT
)";

#[durable_object]
pub struct RaffleChannel {
    hub: Hub<RaffleLive>,
    env: Env,
    /// Set first thing by destroy; checked after every `await`.
    destroyed: Cell<bool>,
}

impl RaffleChannel {
    fn sql(&self) -> worker::SqlStorage {
        self.hub.state().storage().sql()
    }

    fn db(&self) -> Result<Db> {
        crate::database(&self.env)
    }

    /// The raffle ID, highest version seen and last published view.
    fn meta(&self) -> Option<(String, u64, Option<ParticipantView>)> {
        let mut rows = self
            .sql()
            .exec(
                "SELECT raffle_id, seen_v, last FROM rafi_channel WHERE id = 1",
                None,
            )
            .ok()?
            .raw();
        let row = rows.next()?.ok()?;
        match row.as_slice() {
            [
                SqlStorageValue::String(id),
                SqlStorageValue::Integer(seen),
                last,
            ] => {
                let last = match last {
                    SqlStorageValue::String(json) => serde_json::from_str(json).ok(),
                    _ => None,
                };
                Some((id.clone(), (*seen).max(0) as u64, last))
            }
            _ => None,
        }
    }

    fn save(&self, raffle_id: &str, seen: u64, last: Option<&ParticipantView>) -> Result<()> {
        self.sql().exec(SCHEMA, None)?;
        let last = last.and_then(|v| serde_json::to_string(v).ok());
        self.sql().exec(
            "INSERT INTO rafi_channel (id, raffle_id, seen_v, last) VALUES (1, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET raffle_id = excluded.raffle_id, seen_v = excluded.seen_v,
                 last = COALESCE(excluded.last, rafi_channel.last)",
            vec![
                raffle_id.into(),
                SqlStorageValue::Integer(seen as i64),
                last.map_or(SqlStorageValue::Null, SqlStorageValue::String),
            ],
        )?;
        Ok(())
    }

    /// Reads the raffle and publishes if the rule says so. Returns the decision
    /// and whether phones should still hold sockets.
    async fn refresh(&self, raffle_id: &str) -> Result<(Decision, bool)> {
        if self.destroyed.get() {
            return Ok((Decision::Ignore, false));
        }
        let snapshot = Snapshot::read(&self.db()?, raffle_id)
            .await
            .map_err(|e| worker::Error::RustError(e.to_string()))?;
        // D1 is external I/O: other requests (destroy, say) can run meanwhile.
        if self.destroyed.get() {
            return Ok((Decision::Ignore, false));
        }
        let Some(snapshot) = snapshot else {
            return Ok((Decision::Ignore, false));
        };
        Ok((
            self.apply(raffle_id, &snapshot).await?,
            snapshot.accepts_sockets(),
        ))
    }

    /// Applies the publish rule to a snapshot. Everything up to and including
    /// the fan-out runs without awaiting, so the stored version and the
    /// published view can't diverge.
    async fn apply(&self, raffle_id: &str, snapshot: &Snapshot) -> Result<Decision> {
        let view = snapshot.view();
        let (seen, last) = match self.meta() {
            Some((_, seen, last)) => (seen, last),
            None => (0, None),
        };
        let decision = decide(snapshot.channel_open(), seen, last.as_ref(), &view);
        match decision {
            Decision::Publish => {
                self.save(raffle_id, view.v, Some(&view))?;
                self.hub.publish(&view).await?;
            }
            Decision::Unchanged => self.save(raffle_id, view.v, None)?,
            Decision::Stale { .. } | Decision::Ignore => {}
        }
        Ok(decision)
    }

    async fn ensure_alarm(&self) -> Result<()> {
        let storage = self.hub.state().storage();
        if storage.get_alarm().await?.is_none() {
            storage.set_alarm(REREAD).await?;
        }
        Ok(())
    }

    async fn accept(&self, req: Request, raffle_id: &str) -> Result<Response> {
        if self.destroyed.get() {
            return reject(close::FORBIDDEN, "live updates ended");
        }
        if self.hub.state().get_websockets().len() >= MAX_SOCKETS {
            return reject(close::FORBIDDEN, "too many connections");
        }
        // Brings the stored view up to date first, so the handshake sends the
        // current one; also re-checks that the channel is open.
        let (_, accepts) = self.refresh(raffle_id).await?;
        if !accepts || self.destroyed.get() {
            return reject(close::FORBIDDEN, "live updates ended");
        }
        self.ensure_alarm().await?;
        self.hub.fetch(req).await
    }

    async fn destroy(&self, raffle_id: &str) -> Result<()> {
        self.destroyed.set(true);
        // Phones that slept through the end get the final view before the close.
        if let Ok(Some(snapshot)) = Snapshot::read(&self.db()?, raffle_id).await {
            let view = snapshot.view();
            let (seen, last) = self.meta().map_or((0, None), |(_, s, l)| (s, l));
            if decide(true, seen, last.as_ref(), &view) == Decision::Publish {
                self.hub.publish(&view).await?;
            }
        }
        for ws in self.hub.state().get_websockets() {
            let _ = ws.close(Some(close::FORBIDDEN), Some("live updates ended"));
        }
        let storage = self.hub.state().storage();
        storage.delete_alarm().await?;
        storage.delete_all().await?;
        Ok(())
    }
}

impl DurableObject for RaffleChannel {
    fn new(state: State, env: Env) -> Self {
        let hub = Hub::new(state, HubConfig::default().max_event_bytes(MAX_VIEW_BYTES));
        if let Err(e) = hub.state().storage().sql().exec(SCHEMA, None) {
            worker::console_error!("RaffleChannel: creating the table failed: {e}");
        }
        Self {
            hub,
            env,
            destroyed: Cell::new(false),
        }
    }

    async fn fetch(&self, req: Request) -> Result<Response> {
        let Some(raffle_id) = req.headers().get(RAFFLE_HEADER)? else {
            return Response::error("missing raffle", 400);
        };
        let upgrade = req
            .headers()
            .get("upgrade")?
            .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
        if upgrade {
            return self.accept(req, &raffle_id).await;
        }
        let path = req.url()?.path().to_owned();
        match (req.method(), path.as_str()) {
            (Method::Post, "/nudge") => {
                let (decision, _) = self.refresh(&raffle_id).await?;
                // Ignored nudges are answered with 200 too: the sender has nothing to retry.
                Response::ok(format!("{decision:?}"))
            }
            (Method::Post, "/destroy") => {
                self.destroy(&raffle_id).await?;
                Response::ok("destroyed")
            }
            _ => Response::error("Not Found", 404),
        }
    }

    // The socket handlers are the hub's; the alarm below is the object's own.
    partyline_worker::hub_websocket_handlers!(hub);

    /// The periodic re-read. It reschedules itself only while sockets are open
    /// and the raffle still takes them. Not handed to the hub: Latest channels
    /// never trim by age, so the hub never needs the alarm.
    async fn alarm(&self) -> Result<Response> {
        let Some((raffle_id, _, _)) = self.meta() else {
            return Response::ok("");
        };
        // A failed read throws, so the runtime retries the alarm.
        let (_, accepts) = self.refresh(&raffle_id).await?;
        if accepts && !self.destroyed.get() && !self.hub.state().get_websockets().is_empty() {
            self.hub.state().storage().set_alarm(REREAD).await?;
        }
        Response::ok("")
    }
}
