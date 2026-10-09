//! The `Raffle` virtual object.

use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use rafi_drand::DrandClient;
use rafi_drawing::{
    DrawError,
    drawing::DrawState,
    engine::{BeaconOutcome, Command},
    manifest::CommitDoc,
};
use rafi_platform::{DrawStore, Notifier};
use restate_drand::{AwaitError, Drand};
use restate_sdk::prelude::*;

/// The Restate service name.
pub const SERVICE_NAME: &str = "Raffle";

/// The key under which the draw state is stored.
const STATE: &str = "state";

/// The draw engine for one raffle per key.
pub struct Raffle {
    drand: Drand,
    store: Arc<dyn DrawStore>,
    notifier: Arc<dyn Notifier>,
}

impl Raffle {
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
}

/// A rejected command: terminal (not retried), with an HTTP-ish status code
/// the caller can show.
fn rejected(e: impl std::fmt::Display, code: u16) -> HandlerError {
    TerminalError::new_with_code(code, e.to_string()).into()
}

/// A refusal by the draw rules, with its kind's status code.
fn draw_error(e: DrawError) -> HandlerError {
    let code = e.kind().status();
    rejected(e, code)
}

impl Raffle {
    /// Writes the state to the store, at least once (journaled, retried until
    /// it succeeds: the database is the record), then tells clients, best
    /// effort.
    async fn project(&self, ctx: &ObjectContext<'_>, state: &DrawState) -> HandlerResult<()> {
        let store = &*self.store;
        ctx.run(|| async move {
            let written = store
                .save_draw_state(state)
                .await
                .map_err(HandlerError::from)?;
            if !written {
                // Already stored (a retry), or for another entrant list.
                tracing::debug!(
                    raffle_id = state.raffle_id,
                    version = state.version,
                    "draw state not written"
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
        // stuck notification can't block the raffle's object; the attempts
        // only cover the step itself failing.
        let raffle_id = state.raffle_id.as_str();
        let notifier = &*self.notifier;
        let notified = ctx
            .run(|| async move {
                notifier.notify(raffle_id).await;
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
            tracing::warn!(raffle_id, error = %e, "notifying clients failed");
        }
        Ok(())
    }

    async fn load(ctx: &ObjectContext<'_>) -> HandlerResult<DrawState> {
        ctx.get::<Json<DrawState>>(STATE)
            .await?
            .map(Json::into_inner)
            .ok_or_else(|| draw_error(DrawError::NotFrozen))
    }
}

#[restate_sdk::object(
    name = "Raffle",
    journal_retention = "1h",
    idempotency_retention = "1h"
)]
impl Raffle {
    /// Freezes the entrant list. Idempotent for the same list; a different list is rejected.
    #[handler]
    async fn freeze(&self, ctx: ObjectContext<'_>, doc: Json<CommitDoc>) -> HandlerResult<()> {
        let doc = doc.into_inner();
        if doc.raffle_id != ctx.key() {
            return Err(rejected("raffle ID does not match the object key", 400));
        }
        // The app and this endpoint configure drand separately; the rules
        // refuse a raffle whose draws would come from another chain.
        let existing = ctx
            .get::<Json<DrawState>>(STATE)
            .await?
            .map(Json::into_inner);
        let Some(fresh) =
            DrawState::freeze_or_check(existing.as_ref(), &doc, &self.drand.chain().hash())
                .map_err(draw_error)?
        else {
            return Ok(());
        };
        ctx.set(STATE, Json(fresh.clone()));
        self.project(&ctx, &fresh).await
    }

    /// Draws a winner for one prize unit.
    ///
    /// Journals one clock read, commits to the drand round `lead_secs` later and
    /// projects that commitment *before* the round exists, sleeps durably until
    /// it is published, fetches and verifies it (retried up to the configured
    /// timeout) and settles the draw.
    ///
    /// If the beacon cannot be had, the draw is recorded as stalled and stays
    /// pending on the same round: drawing the unit again re-enters the same
    /// pending draw and fetches that round, so a withholding relay cannot force
    /// a redraw on a new round.
    #[handler]
    async fn draw(&self, ctx: ObjectContext<'_>, cmd: Json<Command>) -> HandlerResult<()> {
        let Command::Draw { unit_id, offline } = cmd.into_inner() else {
            return Err(rejected("expected a draw command", 400));
        };
        let mut state = Self::load(&ctx).await?;
        state
            .check_chain(&self.drand.chain().hash())
            .map_err(draw_error)?;

        // Replay-safe: the clock read is journaled, so a resumed invocation
        // commits to the very same round. A stalled draw for this unit is
        // re-entered on its own round.
        let commitment = self.drand.commit_round(&ctx).await?;
        let due = u64::try_from(commitment.due_at.as_secs()).unwrap_or_default();
        let pending = state
            .begin(&unit_id, commitment.round, due)
            .map_err(draw_error)?;
        ctx.set(STATE, Json(state.clone()));
        self.project(&ctx, &state).await?;

        let outcome = if offline {
            let randomness = ctx
                .run(|| async {
                    let mut buf = [0u8; 32];
                    getrandom::fill(&mut buf).map_err(|e| anyhow!("randomness: {e}"))?;
                    Ok(buf.to_vec())
                })
                .name("offline-randomness")
                .await?;
            let randomness: [u8; 32] = randomness
                .try_into()
                .map_err(|_| rejected("bad randomness", 500))?;
            BeaconOutcome::Offline(randomness)
        } else {
            // Sleeps until the round is due, fetches it (journaled, retried) and
            // verifies it. If it can't be had, the draw stalls on its round.
            match self.drand.await_beacon(&ctx, pending.round).await {
                Ok(beacon) => BeaconOutcome::Verified(beacon),
                Err(AwaitError::Unavailable { reason, .. }) => BeaconOutcome::Unavailable(reason),
                // Cancelled (or a step failed): the draw stays pending on its
                // round. Record why, so the organizer sees the retry, offline and
                // abandon controls right away, then fail the invocation.
                Err(e) => {
                    let reason = if e.is_cancelled() {
                        "draw cancelled".to_owned()
                    } else {
                        format!("draw failed: {e}")
                    };
                    state
                        .conclude(BeaconOutcome::Interrupted(reason))
                        .map_err(draw_error)?;
                    ctx.set(STATE, Json(state.clone()));
                    self.project(&ctx, &state).await?;
                    return Err(e.into_handler_error());
                }
            }
        };
        state.conclude(outcome).map_err(draw_error)?;
        ctx.set(STATE, Json(state.clone()));
        self.project(&ctx, &state).await
    }

    /// Resolves a drawn unit, withdraws a unit, or finishes the raffle.
    ///
    /// Finishing clears the object's state: the app holds the permanent record.
    #[handler]
    async fn resolve(&self, ctx: ObjectContext<'_>, cmd: Json<Command>) -> HandlerResult<()> {
        let mut state = Self::load(&ctx).await?;
        state.apply(&cmd.into_inner()).map_err(draw_error)?;
        ctx.set(STATE, Json(state.clone()));
        self.project(&ctx, &state).await?;
        if state.finished {
            ctx.clear_all();
        }
        Ok(())
    }

    /// The current state (recovery and debugging; screens read the projection).
    #[handler]
    async fn get(&self, ctx: SharedObjectContext<'_>) -> HandlerResult<Json<Option<DrawState>>> {
        Ok(Json(
            ctx.get::<Json<DrawState>>(STATE)
                .await?
                .map(Json::into_inner),
        ))
    }
}
