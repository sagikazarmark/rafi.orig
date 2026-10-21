//! The admin API's server side ([`rafi_api::admin`]): `POST /api/admin/{name}`
//! with JSON arguments, for the signed-in organizer only. [`Admin`] maps each
//! function onto its use case.

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::Path,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use dioxus_clerk::server::ClerkAuth;

use rafi_api::{
    Access, Caller, EntrantPatch, LifecycleCommand, Me, PrizeInput, RaffleAdmin, RaffleDefaults,
    RaffleSettings, RaffleSummary, TermsDocument,
    admin::{self, Admin, ApiError, ItemId, Resolution},
};
use rafi_raffle::{Decision, DrawRequest};

use super::{
    AppState, auth, pages,
    service::{self, Error},
};

type DispatchError = admin::DispatchError<Error>;

pub fn routes() -> Router {
    Router::new().route("/api/admin/{name}", post(call))
}

/// `ClerkAuthLayer` (around the whole router) verifies the session; without one,
/// or without the layer, `auth` is `None` and only dev auth lets the call through.
async fn call(
    Extension(app): Extension<AppState>,
    auth: Option<ClerkAuth>,
    Path(name): Path<String>,
    args: Bytes,
) -> Response {
    let Some(caller) = auth::caller(&app, auth) else {
        return error(StatusCode::UNAUTHORIZED, "Please sign in.");
    };
    match admin::dispatch(&app, &caller, &name, &args).await {
        Ok(value) => ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response(),
        Err(e) => app_error(&name, e),
    }
}

/// The status and the organizer's message for why a call failed.
fn app_error(name: &str, e: DispatchError) -> Response {
    let status = match &e {
        DispatchError::Unknown | DispatchError::App(Error::NotFound) => StatusCode::NOT_FOUND,
        DispatchError::Arguments(_) => StatusCode::BAD_REQUEST,
        DispatchError::App(Error::Refused(r)) if r.invalid() => StatusCode::BAD_REQUEST,
        DispatchError::App(Error::Refused(service::Refusal::Forbidden(_))) => StatusCode::FORBIDDEN,
        DispatchError::App(Error::Refused(_)) => StatusCode::CONFLICT,
        DispatchError::App(Error::Unavailable) => StatusCode::SERVICE_UNAVAILABLE,
        DispatchError::App(Error::Internal(e)) => {
            tracing::error!(function = name, error = %e, "internal error");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
        DispatchError::Result(e) => {
            tracing::error!(function = name, error = %e, "encoding the result");
            return error(StatusCode::INTERNAL_SERVER_ERROR, "internal error");
        }
    };
    error(status, &e.to_string())
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(ApiError::new(message))).into_response()
}

/// Refuses unless the caller's tier allows what `needs`. Running a raffle
/// needs nothing beyond membership; what the gate guards is creating,
/// configuring and deleting raffles ([`Access::Manager`]) and the owner's
/// legal standing and data ([`Access::Admin`]).
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
            terms_accepted: service::terms_accepted(self, &c.owner).await?,
            retention_stale: service::housekeeping::stale(self).await?,
        })
    }

    async fn get_terms(&self, c: &Caller) -> service::Result<TermsDocument> {
        Ok(TermsDocument {
            version: self.cfg.terms_version.clone(),
            html: pages::markdown(&self.cfg.documents.terms),
            accepted: service::terms_accepted(self, &c.owner).await?,
        })
    }

    async fn accept_terms(&self, c: &Caller, version: String) -> service::Result<()> {
        allow(c, Access::Admin)?;
        service::accept_terms(self, &c.owner, &c.user, &version).await
    }

    async fn list_raffles(&self, c: &Caller) -> service::Result<Vec<RaffleSummary>> {
        service::list(self, &c.owner).await
    }

    async fn get_defaults(&self, c: &Caller) -> service::Result<RaffleDefaults> {
        service::get_defaults(self, &c.owner).await
    }

    async fn save_defaults(&self, c: &Caller, defaults: RaffleDefaults) -> service::Result<()> {
        allow(c, Access::Manager)?;
        service::save_defaults(self, &c.owner, defaults).await
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
        service::create(self, &c.owner, &title, &event).await
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
        service::update_settings(self, &c.owner, &id, settings).await
    }

    async fn delete_raffle(&self, c: &Caller, id: String) -> service::Result<()> {
        allow(c, Access::Manager)?;
        service::delete(self, &c.owner, &id).await
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
        service::duplicate(self, &c.owner, &id, &event).await
    }

    async fn save_prize(
        &self,
        c: &Caller,
        id: String,
        prize: PrizeInput,
    ) -> service::Result<String> {
        service::upsert_prize(self, &c.owner, &id, prize).await
    }

    async fn delete_prize(&self, c: &Caller, id: String, prize_id: String) -> service::Result<()> {
        service::delete_prize(self, &c.owner, &id, &prize_id).await
    }

    async fn lifecycle(
        &self,
        c: &Caller,
        id: String,
        action: LifecycleCommand,
    ) -> service::Result<()> {
        service::lifecycle(self, &c.owner, &id, action).await
    }

    async fn start_draw(&self, c: &Caller, id: String) -> service::Result<()> {
        service::start_draw(self, &c.owner, &id).await
    }

    async fn draw_item(
        &self,
        c: &Caller,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, &c.owner, &id, DrawRequest { item }, &command_id).await
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
        service::command(self, &c.owner, &id, decision, &command_id).await
    }

    async fn void_item(
        &self,
        c: &Caller,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, &c.owner, &id, Decision::Void { item }, &command_id).await
    }

    async fn abandon_draw(
        &self,
        c: &Caller,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, &c.owner, &id, Decision::Abandon { item }, &command_id).await
    }

    async fn finish_raffle(
        &self,
        c: &Caller,
        id: String,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, &c.owner, &id, Decision::Finish, &command_id).await
    }

    async fn end_live_updates(&self, c: &Caller, id: String) -> service::Result<()> {
        service::end_live(self, &c.owner, &id).await
    }

    async fn add_entrant(
        &self,
        c: &Caller,
        id: String,
        name: String,
        email: Option<String>,
    ) -> service::Result<String> {
        service::add_manual(self, &c.owner, &id, &name, email).await
    }

    async fn add_fake_entrants(&self, c: &Caller, id: String, n: u32) -> service::Result<()> {
        service::add_fakes(self, &c.owner, &id, n).await
    }

    async fn patch_entrant(
        &self,
        c: &Caller,
        id: String,
        entrant_id: String,
        patch: EntrantPatch,
    ) -> service::Result<()> {
        service::patch_entrant(self, &c.owner, &id, &entrant_id, patch).await
    }

    async fn remove_entrant(
        &self,
        c: &Caller,
        id: String,
        entrant_id: String,
    ) -> service::Result<()> {
        service::remove_entrant(self, &c.owner, &id, &entrant_id).await
    }

    async fn export_winners(&self, c: &Caller, id: String) -> service::Result<String> {
        service::export_csv(self, &c.owner, &id).await
    }

    async fn purge_raffle(&self, c: &Caller, id: String) -> service::Result<()> {
        service::purge(self, &c.owner, &id).await
    }

    async fn deliver_all(&self, c: &Caller, id: String) -> service::Result<u64> {
        service::deliver_all(self, &c.owner, &id).await
    }

    async fn remove_my_data(&self, c: &Caller) -> service::Result<()> {
        allow(c, Access::Admin)?;
        service::remove_owner_data(self, &c.owner).await
    }
}
