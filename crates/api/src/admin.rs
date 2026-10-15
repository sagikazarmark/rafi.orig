//! The admin API: plain JSON over HTTP, for the signed-in organizer.
//!
//! [`admin_functions!`](crate::admin_functions) declares every function
//! once: its name, arguments and result. A call POSTs the arguments as a
//! JSON object (field names = parameter names) to `/api/admin/<name>`
//! ([`PATH`]) and gets the JSON-encoded result back, or an [`ApiError`]
//! with a non-2xx status (401 without a session).
//!
//! From that one list come the server's side, [`Admin`] (one method per
//! function, so a server that misses one doesn't compile) and [`dispatch`],
//! and the browser's client, which the web app generates by passing its own
//! macro to `admin_functions!`.

use std::{fmt, future::Future};

pub use rafi_raffle::{ItemId, Resolution};
use serde::{Deserialize, Serialize};

/// Where the admin functions are: `/api/admin/<name>`.
pub const PATH: &str = "/api/admin/";

/// Calls `$callback!` with every admin function, as
/// `fn name(arg: Type, ...) -> Result;`, each with its doc comment.
#[macro_export]
macro_rules! admin_functions {
    ($callback:ident) => {
        $callback! {
            /// Who is signed in, and whether erasure looks stopped.
            fn whoami() -> $crate::Me;
            /// The organizer's raffles.
            fn list_raffles() -> Vec<$crate::RaffleSummary>;
            /// A new draft raffle; returns its ID.
            fn create_raffle(title: String) -> String;
            /// Everything the admin page shows of raffle `id`.
            fn get_raffle(id: String) -> $crate::RaffleAdmin;
            fn save_settings(id: String, settings: $crate::RaffleSettings) -> ();
            fn delete_raffle(id: String) -> ();
            /// A copy of raffle `id` as a new draft; returns its ID.
            fn duplicate_raffle(id: String) -> String;
            /// Adds or edits a prize; returns its ID.
            fn save_prize(id: String, prize: $crate::PrizeInput) -> String;
            fn delete_prize(id: String, prize_id: String) -> ();
            fn lifecycle(id: String, action: $crate::LifecycleCommand) -> ();
            /// Freezes the entrants and hands the raffle to the engine.
            fn start_draw(id: String) -> ();
            fn draw_item(id: String, item: $crate::admin::ItemId, command_id: String) -> ();
            fn resolve_item(
                id: String,
                item: $crate::admin::ItemId,
                resolution: $crate::admin::Resolution,
                command_id: String,
            ) -> ();
            fn void_item(id: String, item: $crate::admin::ItemId, command_id: String) -> ();
            fn abandon_draw(id: String, item: $crate::admin::ItemId, command_id: String) -> ();
            fn finish_raffle(id: String, command_id: String) -> ();
            fn end_live_updates(id: String) -> ();
            /// Adds an entrant by hand; returns their ID.
            fn add_entrant(id: String, name: String, email: Option<String>) -> String;
            fn add_fake_entrants(id: String, n: u32) -> ();
            fn patch_entrant(id: String, entrant_id: String, patch: $crate::EntrantPatch) -> ();
            fn remove_entrant(id: String, entrant_id: String) -> ();
            /// The winners as CSV.
            fn export_winners(id: String) -> String;
            /// Erases the raffle's personal data now.
            fn purge_raffle(id: String) -> ();
        }
    };
}

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

/// Why [`dispatch`] didn't return a result.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError<E> {
    /// No admin function by that name.
    #[error("no such function")]
    Unknown,
    /// The arguments don't decode.
    #[error("Invalid arguments: {0}")]
    Arguments(String),
    /// The result doesn't encode.
    #[error("the result doesn't encode: {0}")]
    Result(serde_json::Error),
    /// The function failed.
    #[error(transparent)]
    App(E),
}

macro_rules! server_side {
    ($(
        $(#[$meta:meta])*
        fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty;
    )*) => {
        /// The server's side of the admin API: every function, for the
        /// organizer `owner`.
        pub trait Admin: Sync {
            type Error;

            $(
                $(#[$meta])*
                fn $name(
                    &self,
                    owner: &str,
                    $($arg: $ty),*
                ) -> impl Future<Output = Result<$ret, Self::Error>> + Send;
            )*
        }

        /// Runs the admin function `name` for `owner` with JSON-encoded `args`
        /// (an empty body counts as `{}`) and returns its JSON-encoded result.
        pub async fn dispatch<A: Admin>(
            api: &A,
            owner: &str,
            name: &str,
            args: &[u8],
        ) -> Result<serde_json::Value, DispatchError<A::Error>> {
            let args = if args.is_empty() { b"{}" } else { args };
            match name {
                $(
                    stringify!($name) => {
                        #[derive(Deserialize)]
                        struct Args { $($arg: $ty),* }
                        let Args { $($arg),* } = serde_json::from_slice(args)
                            .map_err(|e| DispatchError::Arguments(e.to_string()))?;
                        let result = api.$name(owner, $($arg),*).await.map_err(DispatchError::App)?;
                        serde_json::to_value(result).map_err(DispatchError::Result)
                    }
                )*
                _ => Err(DispatchError::Unknown),
            }
        }
    };
}

admin_functions!(server_side);
