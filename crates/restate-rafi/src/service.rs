//! The `Raffle` virtual object.

use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use rafi_core::{
    commit::CommitDoc,
    engine::Command,
    state::{DrawError, DrawState},
};
use rafi_drand::DrandClient;
use restate_drand::{AwaitError, Drand};
use restate_sdk::prelude::*;

use crate::projector::Projector;

/// The Restate service name.
pub const SERVICE_NAME: &str = "Raffle";

/// The key under which the draw state is stored.
const STATE: &str = "state";

/// The draw engine for one raffle per key.
pub struct Raffle {
    drand: Drand,
    projector: Arc<dyn Projector>,
}

impl Raffle {
    pub fn new(drand: DrandClient, projector: impl Projector) -> Self {
        let config = drand.config();
        let drand = Drand::new(drand.client().clone())
            .with_lead(Duration::from_secs(config.lead_secs))
            .with_fetch_budget(Duration::from_secs(config.timeout_secs));
        Self {
            drand,
            projector: Arc::new(projector),
        }
    }
}

/// A rejected command: terminal (not retried), with an HTTP-ish status code
/// the caller can show.
fn rejected(e: impl std::fmt::Display, code: u16) -> HandlerError {
    TerminalError::new_with_code(code, e.to_string()).into()
}

fn draw_error(e: DrawError) -> HandlerError {
    let code = match e {
        DrawError::UnknownUnit(_) => 404,
        DrawError::BadOutcome
        | DrawError::BadListHash
        | DrawError::WrongRound { .. }
        | DrawError::WrongChain => 400,
        _ => 409,
    };
    rejected(e, code)
}

impl Raffle {
    /// Hands the state to the projector, at least once (journaled, retried).
    async fn project(&self, ctx: &ObjectContext<'_>, state: &DrawState) -> HandlerResult<()> {
        let projector = &*self.projector;
        ctx.run(|| async move {
            projector.project(state).await.map_err(HandlerError::from)?;
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
        Ok(())
    }

    async fn load(ctx: &ObjectContext<'_>) -> HandlerResult<DrawState> {
        ctx.get::<Json<DrawState>>(STATE)
            .await?
            .map(Json::into_inner)
            .ok_or_else(|| rejected("raffle is not frozen", 404))
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
        // The app and this endpoint configure drand separately; refuse a raffle
        // whose draws would come from a different chain than the one committed.
        if !doc
            .drand_chain
            .eq_ignore_ascii_case(&self.drand.chain().hash().to_string())
        {
            return Err(rejected(
                format!(
                    "the raffle commits to drand chain {}, but the draw engine uses {}",
                    doc.drand_chain,
                    self.drand.chain().hash()
                ),
                409,
            ));
        }
        let fresh = DrawState::freeze(&doc);
        if let Some(Json(existing)) = ctx.get::<Json<DrawState>>(STATE).await? {
            if existing.list_hash == fresh.list_hash {
                return Ok(());
            }
            return Err(rejected(
                "raffle is already frozen with a different entrant list",
                409,
            ));
        }
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
        if !state
            .drand_chain
            .eq_ignore_ascii_case(&self.drand.chain().hash().to_string())
        {
            return Err(rejected(
                "the raffle commits to a different drand chain than the draw engine uses",
                409,
            ));
        }

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

        if offline {
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
            state.complete_offline(randomness).map_err(draw_error)?;
        } else {
            // Sleeps until the round is due, fetches it (journaled, retried) and
            // verifies it. If it can't be had, the draw stalls on its round.
            match self.drand.await_beacon(&ctx, pending.round).await {
                Ok(beacon) => {
                    if let Err(e) = state.complete(&beacon) {
                        state
                            .stall(&format!("invalid beacon: {e}"))
                            .map_err(draw_error)?;
                    }
                }
                Err(AwaitError::Unavailable { reason, .. }) => state
                    .stall(&format!("drand unavailable: {reason}"))
                    .map_err(draw_error)?,
                // Cancelled (or a step failed): the draw stays pending on its
                // round. Record why, so the organizer sees the retry, offline and
                // abandon controls right away, then fail the invocation.
                Err(e) => {
                    let reason = if e.is_cancelled() {
                        "draw cancelled".to_owned()
                    } else {
                        format!("draw failed: {e}")
                    };
                    state.stall(&reason).map_err(draw_error)?;
                    ctx.set(STATE, Json(state.clone()));
                    self.project(&ctx, &state).await?;
                    return Err(e.into_handler_error());
                }
            }
        }
        ctx.set(STATE, Json(state.clone()));
        self.project(&ctx, &state).await
    }

    /// Resolves a drawn unit, withdraws a unit, or finishes the raffle.
    ///
    /// Finishing clears the object's state: the app holds the permanent record.
    #[handler]
    async fn resolve(&self, ctx: ObjectContext<'_>, cmd: Json<Command>) -> HandlerResult<()> {
        let mut state = Self::load(&ctx).await?;
        match cmd.into_inner() {
            Command::Resolve { unit_id, outcome } => state.resolve(&unit_id, outcome),
            Command::Void { unit_id } => state.void(&unit_id),
            Command::Abandon { unit_id } => state.abandon_unit(&unit_id),
            Command::Finish => state.finish(),
            Command::Draw { .. } => return Err(rejected("use the draw handler", 400)),
        }
        .map_err(draw_error)?;
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
