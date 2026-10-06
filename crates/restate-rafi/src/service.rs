//! The `Raffle` virtual object.

use std::{
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use anyhow::anyhow;
use rafi_core::{
    commit::CommitDoc,
    drand::{round_time, target_round},
    engine::Command,
    state::{DrawError, DrawState},
};
use rafi_drand::{DrandClient, Fetch};
use restate_ext::ContextClockExt;
use restate_sdk::prelude::*;

use crate::projector::Projector;

/// The Restate service name.
pub const SERVICE_NAME: &str = "Raffle";

/// The key under which the draw state is stored.
const STATE: &str = "state";

/// The draw engine for one raffle per key.
pub struct Raffle {
    drand: DrandClient,
    projector: Arc<dyn Projector>,
}

impl Raffle {
    pub fn new(drand: DrandClient, projector: impl Projector) -> Self {
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
        DrawError::BadOutcome | DrawError::BadListHash | DrawError::Beacon(_) => 400,
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
    /// it is published, fetches it (retried up to the configured timeout) and
    /// settles the draw. A failed fetch is recorded as a failed draw; the unit
    /// goes back to pending.
    #[handler]
    async fn draw(&self, ctx: ObjectContext<'_>, cmd: Json<Command>) -> HandlerResult<()> {
        let Command::Draw { unit_id, offline } = cmd.into_inner() else {
            return Err(rejected("expected a draw command", 400));
        };
        let mut state = Self::load(&ctx).await?;
        let config = self.drand.config();

        // Replay-safe: a resumed invocation reads the journaled time, so it
        // commits to the very same round.
        let now = ctx
            .now()
            .await?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let round = target_round(now, config.lead_secs);
        let pending = state
            .begin(&unit_id, round, round_time(round))
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
            let wait = round_time(pending.round).saturating_sub(now);
            if wait > 0 {
                ctx.sleep(Duration::from_millis(wait * 1000 + 300)).await?;
            }
            let drand = &self.drand;
            let fetched = ctx
                .run(|| async move {
                    match drand
                        .fetch(pending.round)
                        .await
                        .map_err(HandlerError::from)?
                    {
                        Fetch::Ready(b) => Ok(Json(b)),
                        Fetch::NotYet => {
                            Err(anyhow!("round {} not published yet", pending.round).into())
                        }
                    }
                })
                .name("beacon")
                .retry_policy(
                    RunRetryPolicy::new()
                        .initial_delay(Duration::from_millis(250))
                        .exponentiation_factor(2.0)
                        .max_delay(Duration::from_secs(2))
                        .max_duration(Duration::from_secs(config.timeout_secs)),
                )
                .await;
            match fetched {
                Ok(Json(beacon)) => {
                    if let Err(e) = state.complete(&beacon) {
                        state.fail(&e.to_string()).map_err(draw_error)?;
                    }
                }
                Err(e) => state
                    .fail(&format!("drand unavailable: {e}"))
                    .map_err(draw_error)?,
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
