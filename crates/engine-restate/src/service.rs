//! The `Drawing` virtual object.

use std::{sync::Arc, time::Duration};

use rafi_drand::DrandClient;
use rafi_drawing::{
    BeaconFetch, BeaconSource, Decision, DrawRequest, Drawing as State, Error as DrawingError,
    ErrorKind, Manifest,
};
use rafi_engine::{DrawStore, NOT_OPEN, status_code};
use rafi_platform::Notifier;
use restate_drand::{AwaitError, Drand};
use restate_sdk::prelude::*;

/// The Restate service name.
pub const SERVICE_NAME: &str = "Drawing";

/// The key under which the drawing is stored.
const STATE: &str = "state";

/// The drawing engine for one drawing per key.
pub struct Drawing {
    drand: Drand,
    store: Arc<dyn DrawStore>,
    notifier: Arc<dyn Notifier>,
}

impl Drawing {
    /// Draws with `drand`, writes every state to `store` and tells clients
    /// through `notifier`.
    pub fn new(drand: DrandClient, store: impl DrawStore, notifier: impl Notifier) -> Self {
        let config = drand.config();
        let drand = Drand::new(drand.client().clone())
            .with_lead(Duration::from_secs(config.lead_secs))
            .with_fetch_budget(Duration::from_secs(config.timeout_secs));
        Self {
            drand,
            store: Arc::new(store),
            notifier: Arc::new(notifier),
        }
    }

    /// Where this engine's randomness comes from.
    fn beacon(&self) -> BeaconSource {
        BeaconSource::Drand(self.drand.chain().hash())
    }
}

/// A rejected command: terminal (not retried), with an HTTP-ish status code
/// the caller can show.
fn rejected(e: impl std::fmt::Display, code: u16) -> HandlerError {
    TerminalError::new_with_code(code, e.to_string()).into()
}

/// A refusal by the draw rules, with its kind's status code.
fn draw_error(e: DrawingError) -> HandlerError {
    let code = status_code(e.kind());
    rejected(e, code)
}

impl Drawing {
    /// Writes the drawing to the store, at least once (journaled, retried until
    /// it succeeds: the database is the record), then tells clients, best
    /// effort.
    async fn project(&self, ctx: &ObjectContext<'_>, state: &State) -> HandlerResult<()> {
        let store = &*self.store;
        ctx.run(|| async move {
            let written = store
                .save_drawing(state)
                .await
                .map_err(HandlerError::from)?;
            if !written {
                // Already stored (a retry), or for another manifest.
                tracing::debug!(
                    drawing_id = %state.id(),
                    version = state.version(),
                    "drawing not written"
                );
            }
            Ok(())
        })
        .name("project")
        .retry_policy(
            RunRetryPolicy::new()
                .initial_delay(Duration::from_millis(250))
                .exponentiation_factor(2.0)
                .max_delay(Duration::from_secs(5)),
        )
        .await?;
        // Its own journaled step, so a replay doesn't notify again. The notifier
        // logs and drops its own failures (clients re-read on their own), so a
        // stuck notification can't block the drawing's object; the attempts
        // only cover the step itself failing. A drawing is keyed by its
        // raffle's id (`rafi_core::drawing::manifest`), so it names the raffle.
        let drawing_id = state.id().as_str();
        let notifier = &*self.notifier;
        let notified = ctx
            .run(|| async move {
                notifier.notify(drawing_id).await;
                Ok(())
            })
            .name("notify")
            .retry_policy(
                RunRetryPolicy::new()
                    .initial_delay(Duration::from_millis(100))
                    .max_attempts(3),
            )
            .await;
        if let Err(e) = notified {
            tracing::warn!(drawing_id, error = %e, "notifying clients failed");
        }
        Ok(())
    }

    async fn load(ctx: &ObjectContext<'_>) -> HandlerResult<State> {
        ctx.get::<Json<State>>(STATE)
            .await?
            .map(Json::into_inner)
            .ok_or_else(|| rejected(NOT_OPEN, status_code(ErrorKind::NotFound)))
    }
}

#[restate_sdk::object(
    name = "Drawing",
    journal_retention = "1h",
    idempotency_retention = "1h"
)]
impl Drawing {
    /// Opens the drawing from its manifest. Idempotent for the same manifest;
    /// another manifest is rejected.
    #[handler]
    async fn open(&self, ctx: ObjectContext<'_>, doc: Json<Manifest>) -> HandlerResult<()> {
        let doc = doc.into_inner();
        if doc.drawing.as_str() != ctx.key() {
            return Err(rejected("drawing ID does not match the object key", 400));
        }
        // The app and this endpoint configure drand separately; the rules
        // refuse a drawing whose draws would come from another source.
        let existing = ctx.get::<Json<State>>(STATE).await?.map(Json::into_inner);
        let Some(fresh) =
            State::open_or_check(existing.as_ref(), &doc, &self.beacon()).map_err(draw_error)?
        else {
            return Ok(());
        };
        ctx.set(STATE, Json(fresh.clone()));
        self.project(&ctx, &fresh).await
    }

    /// Draws a winner for one prize item.
    ///
    /// Journals one clock read, commits to the drand round `lead_secs` later and
    /// projects that commitment *before* the round exists, sleeps durably until
    /// it is published, fetches and verifies it (retried up to the configured
    /// timeout) and settles the draw.
    ///
    /// If the beacon cannot be had, the draw is recorded as stalled and stays
    /// pending on the same round: drawing the item again re-enters the same
    /// pending draw and fetches that round, so a withholding relay cannot force
    /// a redraw on a new round.
    #[handler]
    async fn draw(&self, ctx: ObjectContext<'_>, request: Json<DrawRequest>) -> HandlerResult<()> {
        let request = request.into_inner();
        let mut state = Self::load(&ctx).await?;

        // Replay-safe: the clock read is journaled, so a resumed invocation
        // commits to the very same round. A stalled draw for this item is
        // re-entered on its own round.
        let commitment = self.drand.commit_round(&ctx).await?;
        let due = u64::try_from(commitment.due_at.as_secs()).unwrap_or_default();
        let pending = state
            .announce(&request, &self.beacon(), commitment.round, due)
            .map_err(draw_error)?;
        ctx.set(STATE, Json(state.clone()));
        self.project(&ctx, &state).await?;

        // Sleeps until the round is due, fetches it (journaled, retried) and
        // verifies it. If it can't be had, the draw stalls on its round.
        let outcome = match self.drand.await_beacon(&ctx, pending.round).await {
            Ok(beacon) => BeaconFetch::Verified(beacon),
            Err(AwaitError::Unavailable { reason, .. }) => BeaconFetch::Unavailable(reason),
            // Cancelled (or a step failed): the draw stays pending on its
            // round. Record why, so the organizer sees the retry and abandon
            // controls right away, then fail the invocation.
            Err(e) => {
                let reason = if e.is_cancelled() {
                    "draw cancelled".to_owned()
                } else {
                    format!("draw failed: {e}")
                };
                state
                    .settle(BeaconFetch::Interrupted(reason))
                    .map_err(draw_error)?;
                ctx.set(STATE, Json(state.clone()));
                self.project(&ctx, &state).await?;
                return Err(e.into_handler_error());
            }
        };
        state.settle(outcome).map_err(draw_error)?;
        ctx.set(STATE, Json(state.clone()));
        self.project(&ctx, &state).await
    }

    /// Applies a decision: resolves a drawn item, withdraws an item, abandons
    /// the pending draw, or finishes the drawing.
    ///
    /// Finishing clears the object's state: the app holds the permanent record.
    #[handler]
    async fn decide(&self, ctx: ObjectContext<'_>, decision: Json<Decision>) -> HandlerResult<()> {
        let mut state = Self::load(&ctx).await?;
        state.decide(&decision.into_inner()).map_err(draw_error)?;
        ctx.set(STATE, Json(state.clone()));
        self.project(&ctx, &state).await?;
        if state.is_finished() {
            ctx.clear_all();
        }
        Ok(())
    }

    /// The current drawing (recovery and debugging; screens read the projection).
    #[handler]
    async fn get(&self, ctx: SharedObjectContext<'_>) -> HandlerResult<Json<Option<State>>> {
        Ok(Json(
            ctx.get::<Json<State>>(STATE).await?.map(Json::into_inner),
        ))
    }
}
