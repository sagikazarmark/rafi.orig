//! The admin API's server side ([`rafi_model::api`]): `POST /api/admin/{name}`
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

use rafi_model::{
    EntrantPatch, LifecycleCommand, Me, PrizeInput, RaffleAdmin, RaffleSettings, RaffleSummary,
    api::{self, Admin, ApiError, ItemId, Resolution},
};
use rafi_raffle::{Decision, DrawRequest};

use super::{
    AppState, auth,
    service::{self, Error},
};

type DispatchError = api::DispatchError<Error>;

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
    let Some(owner) = auth::owner(&app, auth) else {
        return error(StatusCode::UNAUTHORIZED, "Please sign in.");
    };
    match api::dispatch(&app, &owner, &name, &args).await {
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

impl Admin for AppState {
    type Error = Error;

    async fn whoami(&self, owner: &str) -> service::Result<Me> {
        Ok(Me {
            owner_id: owner.to_owned(),
            dev_auth: self.cfg.dev_auth,
            retention_stale: service::retention::stale(self).await?,
        })
    }

    async fn list_raffles(&self, owner: &str) -> service::Result<Vec<RaffleSummary>> {
        service::list(self, owner).await
    }

    async fn create_raffle(&self, owner: &str, title: String) -> service::Result<String> {
        service::create(self, owner, &title).await
    }

    async fn get_raffle(&self, owner: &str, id: String) -> service::Result<RaffleAdmin> {
        service::admin(self, owner, &id).await
    }

    async fn save_settings(
        &self,
        owner: &str,
        id: String,
        settings: RaffleSettings,
    ) -> service::Result<()> {
        service::update_settings(self, owner, &id, settings).await
    }

    async fn delete_raffle(&self, owner: &str, id: String) -> service::Result<()> {
        service::delete(self, owner, &id).await
    }

    async fn duplicate_raffle(&self, owner: &str, id: String) -> service::Result<String> {
        service::duplicate(self, owner, &id).await
    }

    async fn save_prize(
        &self,
        owner: &str,
        id: String,
        prize: PrizeInput,
    ) -> service::Result<String> {
        service::upsert_prize(self, owner, &id, prize).await
    }

    async fn delete_prize(&self, owner: &str, id: String, prize_id: String) -> service::Result<()> {
        service::delete_prize(self, owner, &id, &prize_id).await
    }

    async fn lifecycle(
        &self,
        owner: &str,
        id: String,
        action: LifecycleCommand,
    ) -> service::Result<()> {
        service::lifecycle(self, owner, &id, action).await
    }

    async fn start_draw(&self, owner: &str, id: String) -> service::Result<()> {
        service::start_draw(self, owner, &id).await
    }

    async fn draw_item(
        &self,
        owner: &str,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, owner, &id, DrawRequest { item }, &command_id).await
    }

    async fn resolve_item(
        &self,
        owner: &str,
        id: String,
        item: ItemId,
        resolution: Resolution,
        command_id: String,
    ) -> service::Result<()> {
        let decision = Decision::Resolve { item, resolution };
        service::command(self, owner, &id, decision, &command_id).await
    }

    async fn void_item(
        &self,
        owner: &str,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, owner, &id, Decision::Void { item }, &command_id).await
    }

    async fn abandon_draw(
        &self,
        owner: &str,
        id: String,
        item: ItemId,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, owner, &id, Decision::Abandon { item }, &command_id).await
    }

    async fn finish_raffle(
        &self,
        owner: &str,
        id: String,
        command_id: String,
    ) -> service::Result<()> {
        service::command(self, owner, &id, Decision::Finish, &command_id).await
    }

    async fn end_live_updates(&self, owner: &str, id: String) -> service::Result<()> {
        service::end_live(self, owner, &id).await
    }

    async fn add_entrant(
        &self,
        owner: &str,
        id: String,
        name: String,
        email: Option<String>,
    ) -> service::Result<String> {
        service::add_manual(self, owner, &id, &name, email).await
    }

    async fn add_fake_entrants(&self, owner: &str, id: String, n: u32) -> service::Result<()> {
        service::add_fakes(self, owner, &id, n).await
    }

    async fn patch_entrant(
        &self,
        owner: &str,
        id: String,
        entrant_id: String,
        patch: EntrantPatch,
    ) -> service::Result<()> {
        service::patch_entrant(self, owner, &id, &entrant_id, patch).await
    }

    async fn remove_entrant(
        &self,
        owner: &str,
        id: String,
        entrant_id: String,
    ) -> service::Result<()> {
        service::remove_entrant(self, owner, &id, &entrant_id).await
    }

    async fn export_winners(&self, owner: &str, id: String) -> service::Result<String> {
        service::export_csv(self, owner, &id).await
    }

    async fn purge_raffle(&self, owner: &str, id: String) -> service::Result<()> {
        service::purge(self, owner, &id).await
    }
}
