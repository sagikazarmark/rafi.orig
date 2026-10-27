//! The admin API's server side ([`rafi_api::admin`]): `POST /api/admin/{name}`
//! with JSON arguments, for the signed-in organizer only. [`Admin`] maps each
//! function onto its use case.

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::Path,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use dioxus_clerk::server::ClerkAuth;

use rafi_api::{
    Access, Caller, EntrantPatch, LifecycleCommand, Me, PrizeInput, RaffleAdmin, RaffleDefaults,
    RaffleSettings, RaffleSummary,
    admin::{self, Admin, ApiError, ItemId, Resolution, codes},
};
use rafi_raffle::{Decision, DrawRequest};

use super::{
    AppState, auth,
    service::{self, Error},
};

type DispatchError = admin::DispatchError<Error>;

pub fn routes() -> Router {
    Router::new().route("/api/admin/{name}", post(call))
}

/// `ClerkAuthLayer` (around the whole router) verifies the session; without one,
/// or without the layer, `auth` is `None` and only dev auth lets the call through.
///
/// The arguments must come as `application/json`. The session rides in a
/// cookie, so a page on another site could make the browser POST here; a
/// cross-site form can't set that content type (only `text/plain`,
/// `multipart/form-data` or `application/x-www-form-urlencoded`), while
/// `fetch` with JSON from another origin is stopped by CORS. With Clerk's
/// `SameSite=Lax` cookie this is a second line, not the first.
async fn call(
    Extension(app): Extension<AppState>,
    auth: Option<ClerkAuth>,
    Path(name): Path<String>,
    headers: HeaderMap,
    args: Bytes,
) -> Response {
    let Some(caller) = auth::caller(&app, auth) else {
        return error(
            StatusCode::UNAUTHORIZED,
            codes::UNAUTHORIZED,
            "Please sign in.",
        );
    };
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"))
        });
    if !json && !args.is_empty() {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            codes::INVALID_ARGUMENTS,
            "The arguments must be sent as application/json.",
        );
    }
    match admin::dispatch(&app, &caller, &name, &args).await {
        Ok(value) => ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response(),
        Err(e) => app_error(&name, e),
    }
}

/// The status, the code and the organizer's message for why a call failed.
/// A refusal's code is its kind ([`service::Refusal::code`]); the status
/// says what kind of failure: 400 for a request that can never succeed, 403
/// for the caller's tier, 404 for what isn't theirs, 409 for what the
/// raffle's state refuses, 503 for an engine out of reach.
fn app_error(name: &str, e: DispatchError) -> Response {
    let (status, code) = match &e {
        DispatchError::Unknown | DispatchError::App(Error::NotFound) => {
            (StatusCode::NOT_FOUND, codes::NOT_FOUND)
        }
        DispatchError::Arguments(_) => (StatusCode::BAD_REQUEST, codes::INVALID_ARGUMENTS),
        DispatchError::App(Error::Refused(r)) if r.invalid() => (StatusCode::BAD_REQUEST, r.code()),
        DispatchError::App(Error::Refused(r @ service::Refusal::Forbidden(_))) => {
            (StatusCode::FORBIDDEN, r.code())
        }
        DispatchError::App(Error::Refused(r)) => (StatusCode::CONFLICT, r.code()),
        DispatchError::App(Error::Unavailable) => {
            (StatusCode::SERVICE_UNAVAILABLE, codes::UNAVAILABLE)
        }
        DispatchError::App(Error::Internal(e)) => {
            tracing::error!(function = name, error = %e, "internal error");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                codes::INTERNAL,
                "internal error",
            );
        }
        DispatchError::Result(e) => {
            tracing::error!(function = name, error = %e, "encoding the result");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                codes::INTERNAL,
                "internal error",
            );
        }
    };
    error(status, code, &e.to_string())
}

/// Error replies are `no-store` like the successes: nothing of the admin API
/// belongs in a cache.
fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(ApiError::new(code, message)),
    )
        .into_response()
}

/// Refuses unless the caller's tier allows what `needs`. Running a raffle
/// needs nothing beyond membership; what the gate guards is creating,
/// configuring and deleting raffles ([`Access::Manager`]) and removing the
/// owner's data ([`Access::Admin`]).
fn allow(c: &Caller, needs: Access) -> service::Result<()> {
    if c.access.allows(needs) {
        Ok(())
    } else {
        Err(service::Refusal::Forbidden(needs).into())
    }
}

impl Admin for AppState {
    type Error = Error;

    async fn whoami(&self, c: &Caller) -> service::Result<Me> {
        Ok(Me {
            owner_id: c.owner.clone(),
            dev_auth: self.cfg.dev_auth,
            access: c.access,
            shared: c.owner != c.user,
            retention_stale: service::housekeeping::stale(self).await?,
        })
    }

    async fn list_raffles(&self, c: &Caller) -> service::Result<Vec<RaffleSummary>> {
        service::list(self, &c.owner).await
    }

    async fn get_defaults(&self, c: &Caller) -> service::Result<RaffleDefaults> {
        service::get_defaults(self, &c.owner).await
    }

    async fn save_defaults(&self, c: &Caller, defaults: RaffleDefaults) -> service::Result<()> {
        allow(c, Access::Manager)?;
        service::save_defaults(self, service::actor(c), defaults).await
    }

    async fn create_raffle(
        &self,
        c: &Caller,
        title: String,
        event_date: String,
        event_ends_at: Option<i64>,
    ) -> service::Result<String> {
        allow(c, Access::Manager)?;
        let event = service::EventDate {
            date: event_date,
            ends_at: event_ends_at,
        };
        service::create(self, service::actor(c), &title, &event).await
    }

    async fn get_raffle(&self, c: &Caller, id: String) -> service::Result<RaffleAdmin> {
        service::admin(self, &c.owner, &id).await
    }

    async fn save_settings(
        &self,
        c: &Caller,
        id: String,
        settings: RaffleSettings,
    ) -> service::Result<()> {
        allow(c, Access::Manager)?;
        service::update_settings(self, service::actor(c), &id, settings).await
    }

    async fn delete_raffle(&self, c: &Caller, id: String) -> service::Result<()> {
        allow(c, Access::Manager)?;
        service::delete(self, service::actor(c), &id).await
    }

    async fn duplicate_raffle(
        &self,
        c: &Caller,
        id: String,
        event_date: String,
        event_ends_at: Option<i64>,
    ) -> service::Result<String> {
        allow(c, Access::Manager)?;
        let event = service::EventDate {
            date: event_date,
            ends_at: event_ends_at,
        };
        service::duplicate(self, service::actor(c), &id, &event).await
    }

    async fn save_prize(
        &self,
        c: &Caller,
        id: String,
        prize: PrizeInput,
    ) -> service::Result<String> {
        service::upsert_prize(self, service::actor(c), &id, prize).await
    }

    async fn delete_prize(&self, c: &Caller, id: String, prize_id: String) -> service::Result<()> {
        service::delete_prize(self, service::actor(c), &id, &prize_id).await
    }

    async fn set_lifecycle(
        &self,
        c: &Caller,
        id: String,
        action: LifecycleCommand,
    ) -> service::Result<()> {
        service::lifecycle(self, service::actor(c), &id, action).await
    }

    async fn start_draw(&self, c: &Caller, id: String) -> service::Result<()> {
        service::start_draw(self, service::actor(c), &id).await
    }

    async fn draw_item(
        &self,
        c: &Caller,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(
            self,
            service::actor(c),
            &id,
            DrawRequest { item },
            &command_id,
        )
        .await
    }

    async fn resolve_item(
        &self,
        c: &Caller,
        id: String,
        item: ItemId,
        resolution: Resolution,
        command_id: String,
    ) -> service::Result<()> {
        let decision = Decision::Resolve { item, resolution };
        service::command(self, service::actor(c), &id, decision, &command_id).await
    }

    async fn void_item(
        &self,
        c: &Caller,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(
            self,
            service::actor(c),
            &id,
            Decision::Void { item },
            &command_id,
        )
        .await
    }

    async fn abandon_draw(
        &self,
        c: &Caller,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(
            self,
            service::actor(c),
            &id,
            Decision::Abandon { item },
            &command_id,
        )
        .await
    }

    async fn finish_raffle(
        &self,
        c: &Caller,
        id: String,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, service::actor(c), &id, Decision::Finish, &command_id).await
    }

    async fn end_live_updates(&self, c: &Caller, id: String) -> service::Result<()> {
        service::end_live(self, service::actor(c), &id).await
    }

    async fn add_entrant(
        &self,
        c: &Caller,
        id: String,
        name: String,
        email: Option<String>,
    ) -> service::Result<String> {
        service::add_manual(self, service::actor(c), &id, &name, email).await
    }

    async fn add_fake_entrants(&self, c: &Caller, id: String, n: u32) -> service::Result<()> {
        service::add_fakes(self, service::actor(c), &id, n).await
    }

    async fn patch_entrant(
        &self,
        c: &Caller,
        id: String,
        entrant_id: String,
        patch: EntrantPatch,
    ) -> service::Result<()> {
        service::patch_entrant(self, service::actor(c), &id, &entrant_id, patch).await
    }

    async fn remove_entrant(
        &self,
        c: &Caller,
        id: String,
        entrant_id: String,
    ) -> service::Result<()> {
        service::remove_entrant(self, service::actor(c), &id, &entrant_id).await
    }

    async fn export_winners_csv(&self, c: &Caller, id: String) -> service::Result<String> {
        service::export_csv(self, service::actor(c), &id).await
    }

    async fn purge_raffle(&self, c: &Caller, id: String) -> service::Result<()> {
        service::purge(self, service::actor(c), &id).await
    }

    async fn deliver_all_prizes(&self, c: &Caller, id: String) -> service::Result<u64> {
        service::deliver_all(self, service::actor(c), &id).await
    }

    async fn remove_my_data(&self, c: &Caller) -> service::Result<()> {
        allow(c, Access::Admin)?;
        service::remove_owner_data(self, service::actor(c)).await
    }
}
