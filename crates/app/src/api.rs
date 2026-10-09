//! The admin API: plain JSON over HTTP.
//!
//! Each function is declared once in [`admin_api!`]. In the browser (`web`) it is a
//! client that POSTs its arguments as a JSON object (field names = parameter names)
//! to `/api/admin/<name>` and decodes the JSON reply. On the server (`server`) it is
//! an arm of [`dispatch`], which `server::admin` calls with the organizer resolved
//! from the Clerk session (or `dev`); it calls the `service` layer.
//!
//! Errors travel as an [`ApiError`] with a non-2xx status (401 without a session).

use std::fmt;

#[cfg(any(feature = "web", feature = "server"))]
use rafi_drawing::drawing::Outcome;
use serde::{Deserialize, Serialize};

#[cfg(feature = "web")]
use crate::model::VerifyData;
#[cfg(any(feature = "web", feature = "server"))]
use crate::model::{
    EntrantPatch, Lifecycle, Me, PrizeInput, RaffleAdmin, RaffleSettings, RaffleSummary,
};
#[cfg(feature = "server")]
use crate::server::{
    AppState,
    service::{self, AppError},
};
#[cfg(feature = "server")]
use rafi_drawing::engine::Command;

/// An error reply: a message for the organizer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub message: String,
}

impl ApiError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

/// Declares the admin functions: `fn name(args) -> T = |app, owner| future;`, where
/// the future (borrowing `app: &AppState`, `owner: &str` and the arguments) resolves
/// to a `service::Result<T>`.
macro_rules! admin_api {
    ($(
        $(#[$meta:meta])*
        fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty = |$app:ident, $owner:ident| $body:expr;
    )*) => {
        $(
            $(#[$meta])*
            #[cfg(feature = "web")]
            pub async fn $name($($arg: $ty),*) -> Result<$ret, ApiError> {
                #[derive(Serialize)]
                struct Args { $($arg: $ty),* }
                client::post(concat!("/api/admin/", stringify!($name)), &Args { $($arg),* }).await
            }
        )*

        /// Runs the admin function `name` for `owner` with JSON-encoded `args`
        /// (an empty body counts as `{}`) and returns its JSON-encoded result.
        #[cfg(feature = "server")]
        pub async fn dispatch(
            app: &AppState,
            owner: &str,
            name: &str,
            args: &[u8],
        ) -> Result<serde_json::Value, AppError> {
            match name {
                $(
                    stringify!($name) => {
                        #[derive(Deserialize)]
                        struct Args { $($arg: $ty),* }
                        let Args { $($arg),* } = decode(args)?;
                        let ($app, $owner) = (app, owner);
                        $body.await.and_then(encode::<$ret>)
                    }
                )*
                _ => Err(AppError::NotFound),
            }
        }
    };
}

admin_api! {
    fn whoami() -> Me = |app, owner| async {
        Ok(Me {
            owner_id: owner.to_owned(),
            dev_auth: app.cfg.dev_auth,
            retention_enabled: app.cfg.retention_enabled,
            retention_ok_at: crate::server::retention::last_ok(app).await?,
        })
    };

    fn list_raffles() -> Vec<RaffleSummary> = |app, owner| service::list(app, owner);

    fn create_raffle(title: String) -> String = |app, owner| service::create(app, owner, &title);

    fn get_raffle(id: String) -> RaffleAdmin = |app, owner| service::admin(app, owner, &id);

    fn save_settings(id: String, settings: RaffleSettings) -> () =
        |app, owner| service::update_settings(app, owner, &id, settings);

    fn delete_raffle(id: String) -> () = |app, owner| service::delete(app, owner, &id);

    fn duplicate_raffle(id: String) -> String = |app, owner| service::duplicate(app, owner, &id);

    fn save_prize(id: String, prize: PrizeInput) -> String =
        |app, owner| service::upsert_prize(app, owner, &id, prize);

    fn delete_prize(id: String, prize_id: String) -> () =
        |app, owner| service::delete_prize(app, owner, &id, &prize_id);

    fn lifecycle(id: String, action: Lifecycle) -> () =
        |app, owner| service::lifecycle(app, owner, &id, action);

    fn start_draw(id: String) -> () = |app, owner| service::start_draw(app, owner, &id);

    fn draw_unit(id: String, item_id: String, offline: bool, command_id: String) -> () =
        |app, owner| service::command(app, owner, &id, Command::Draw { item_id, offline }, &command_id);

    fn resolve_unit(id: String, item_id: String, outcome: Outcome, command_id: String) -> () =
        |app, owner| service::command(app, owner, &id, Command::Resolve { item_id, outcome }, &command_id);

    fn void_unit(id: String, item_id: String, command_id: String) -> () =
        |app, owner| service::command(app, owner, &id, Command::Void { item_id }, &command_id);

    fn abandon_draw(id: String, item_id: String, command_id: String) -> () =
        |app, owner| service::command(app, owner, &id, Command::Abandon { item_id }, &command_id);

    fn finish_raffle(id: String, command_id: String) -> () =
        |app, owner| service::command(app, owner, &id, Command::Finish, &command_id);

    fn end_live_updates(id: String) -> () = |app, owner| service::end_live(app, owner, &id);

    fn add_entrant(id: String, name: String, email: Option<String>) -> String =
        |app, owner| service::add_manual(app, owner, &id, &name, email);

    fn add_fake_entrants(id: String, n: u32) -> () = |app, owner| service::add_fakes(app, owner, &id, n);

    fn patch_entrant(id: String, entrant_id: String, patch: EntrantPatch) -> () =
        |app, owner| service::patch_entrant(app, owner, &id, &entrant_id, patch);

    fn remove_entrant(id: String, entrant_id: String) -> () =
        |app, owner| service::remove_entrant(app, owner, &id, &entrant_id);

    fn export_winners(id: String) -> String = |app, owner| service::export_csv(app, owner, &id);

    fn purge_raffle(id: String) -> () = |app, owner| service::purge(app, owner, &id);
}

/// Public: data for the verification page (`/api/r/<slug>/verify.json`, no session needed).
#[cfg(feature = "web")]
pub async fn get_verify(slug: String) -> Result<VerifyData, ApiError> {
    let slug = String::from(js_sys::encode_uri_component(&slug));
    client::get(&format!("/api/r/{slug}/verify.json")).await
}

/// Public: a raffle's full view (`/api/r/<slug>/view`), which the projector and
/// the admin poll.
#[cfg(feature = "web")]
pub async fn get_view(slug: String) -> Result<crate::model::RaffleView, ApiError> {
    let slug = String::from(js_sys::encode_uri_component(&slug));
    client::get(&format!("/api/r/{slug}/view")).await
}

#[cfg(feature = "server")]
fn decode<T: serde::de::DeserializeOwned>(args: &[u8]) -> Result<T, AppError> {
    let args = if args.is_empty() { b"{}" } else { args };
    serde_json::from_slice(args)
        .map_err(|e| AppError::BadRequest(format!("Invalid arguments: {e}")))
}

#[cfg(feature = "server")]
fn encode<T: Serialize>(value: T) -> Result<serde_json::Value, AppError> {
    serde_json::to_value(value).map_err(|e| AppError::Internal(e.into()))
}

/// Same-origin requests, so the browser sends Clerk's session cookie.
#[cfg(feature = "web")]
mod client {
    use gloo_net::http::{Request, Response};
    use serde::{Serialize, de::DeserializeOwned};

    use super::ApiError;

    pub async fn post<A: Serialize, T: DeserializeOwned>(
        url: &str,
        args: &A,
    ) -> Result<T, ApiError> {
        let req = Request::post(url).json(args).map_err(network)?;
        reply(req.send().await.map_err(network)?).await
    }

    pub async fn get<T: DeserializeOwned>(url: &str) -> Result<T, ApiError> {
        reply(Request::get(url).send().await.map_err(network)?).await
    }

    fn network(e: gloo_net::Error) -> ApiError {
        ApiError::new(format!("Could not reach the server: {e}"))
    }

    async fn reply<T: DeserializeOwned>(resp: Response) -> Result<T, ApiError> {
        let body = resp.text().await.map_err(network)?;
        if resp.ok() {
            return serde_json::from_str(&body)
                .map_err(|e| ApiError::new(format!("Unexpected reply from the server: {e}")));
        }
        if let Ok(e) = serde_json::from_str::<ApiError>(&body) {
            return Err(e);
        }
        Err(ApiError::new(match resp.status() {
            401 => "Please sign in.".to_owned(),
            404 => "not found".to_owned(),
            status if body.trim().is_empty() => format!("Request failed ({status})."),
            _ => body,
        }))
    }
}
