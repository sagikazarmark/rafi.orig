//! The admin API: plain JSON over HTTP, for the signed-in organizer.
//!
//! [`admin_functions!`](crate::admin_functions) declares every function
//! once: its name, arguments and result. A call POSTs the arguments as a
//! JSON object (field names = parameter names) to `/api/admin/<name>`
//! ([`PATH`]) and gets the JSON-encoded result back, or an [`ApiError`]
//! with a non-2xx status (401 without a session): a machine-readable `code`
//! and a `message` for the organizer.
//!
//! The names are a contract with every client: a verb and its object
//! (`create_raffle`, `save_prize`, `set_lifecycle`), `save` for an upsert,
//! `delete` for what is gone for good, `remove` for an entrant (deleted
//! before the freeze, erased after it), `export_…_csv` for what returns CSV.
//!
//! From that one list come the server's side, [`Admin`] (one method per
//! function, so a server that misses one doesn't compile) and [`dispatch`],
//! and the browser's client, which the web app generates by passing its own
//! macro to `admin_functions!`.

use std::{fmt, future::Future};

pub use rafi_lifecycle::prize::Delivery;
pub use rafi_raffle::{ItemId, Resolution};
use serde::{Deserialize, Serialize};

/// Where the admin functions are: `/api/admin/<name>`.
pub const PATH: &str = "/api/admin/";

/// Calls `$callback!` with every admin function, as
/// `fn name(arg: Type, ...) -> Result;`, each with its doc comment. An
/// argument may carry `serde` attributes (`#[serde(default)]`, so a client
/// that doesn't send it still calls the function), which apply where the
/// arguments are decoded.
#[macro_export]
macro_rules! admin_functions {
    ($callback:ident) => {
        $callback! {
            /// Who is signed in.
            fn whoami() -> $crate::Me;
            /// The organizer's raffles.
            fn list_raffles() -> Vec<$crate::RaffleSummary>;
            /// What the organizer's new raffles start with (empty until saved).
            fn get_defaults() -> $crate::RaffleDefaults;
            fn save_defaults(defaults: $crate::RaffleDefaults) -> ();
            /// A new draft raffle on `event_date` (`YYYY-MM-DD`; `event_ends_at`
            /// is when that day ends where the organizer is, or `None` for
            /// UTC), with the organizer's defaults; returns its ID.
            fn create_raffle(title: String, event_date: String, event_ends_at: Option<i64>) -> String;
            /// Everything the admin page shows of raffle `id`.
            fn get_raffle(id: String) -> $crate::RaffleAdmin;
            fn save_settings(id: String, settings: $crate::RaffleSettings) -> ();
            fn delete_raffle(id: String) -> ();
            /// A copy of raffle `id` as a new draft on `event_date` (as
            /// `create_raffle`); returns its ID.
            fn duplicate_raffle(id: String, event_date: String, event_ends_at: Option<i64>) -> String;
            /// Adds or edits a prize; returns its ID.
            fn save_prize(id: String, prize: $crate::PrizeInput) -> String;
            fn delete_prize(id: String, prize_id: String) -> ();
            /// How a prize reaches its winner. Unlike the rest of a prize, it
            /// can change after the freeze, until the raffle is over; it
            /// applies to the claims made after the change.
            fn set_prize_delivery(
                id: String,
                prize_id: String,
                delivery: $crate::admin::Delivery,
            ) -> ();
            /// Moves the raffle through its lifecycle (publish, open, close, …).
            fn set_lifecycle(id: String, action: $crate::LifecycleCommand) -> ();
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
            /// Adds people the organizer vouches for, one person or a list, all
            /// or nothing; a retry with the same batch ID answers the first
            /// result and adds nothing.
            fn add_entrants(id: String, add: $crate::AddRequest) -> $crate::AddResult;
            /// Takes an add's entrants off the list again, before the freeze;
            /// returns how many (0 if none are left).
            fn undo_add(id: String, batch_id: String) -> u32;
            fn add_fake_entrants(id: String, n: u32) -> ();
            fn patch_entrant(id: String, entrant_id: String, patch: $crate::EntrantPatch) -> ();
            /// Takes an entrant off the list: deleted before the freeze, their
            /// personal data erased after it (once the raffle is over or its
            /// erasure date has passed). `at_request`: they asked; before the
            /// freeze a later add leaves them out unless the organizer lifts it.
            fn remove_entrant(
                id: String,
                entrant_id: String,
                #[serde(default)] at_request: bool,
            ) -> ();
            /// Someone the organizer added takes themselves off, from the freeze
            /// until the raffle is over: their name is hidden, a draw that picks
            /// them can only be declined, and their data is erased once the
            /// raffle is over. Not for a winner who claimed a prize.
            fn object_entrant(id: String, entrant_id: String) -> ();
            /// Undoes `object_entrant` (a mistake), until the raffle is over.
            /// Their name stays hidden until shown again.
            fn unobject_entrant(id: String, entrant_id: String) -> ();
            /// The winners as CSV; every export is audited.
            fn export_winners_csv(id: String) -> String;
            /// Erases the raffle's personal data now.
            fn purge_raffle(id: String) -> ();
            /// Marks every winner's prize delivered: their data goes on the
            /// non-winner clock. Returns how many.
            fn deliver_all_prizes(id: String) -> u64;
            /// Removes the organizer's own data: their defaults, and their
            /// name and contacts from their raffles, whose personal data is
            /// erased. Raffles that were never frozen are deleted.
            fn remove_my_data() -> ();
        }
    };
}

/// An error reply: what went wrong, for a program (`code`, stable
/// `snake_case` names: a refusal's kind, or [`codes`]') and for the
/// organizer (`message`, English).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    #[serde(default)]
    pub code: String,
    pub message: String,
    /// The failed request's ID, for a server error (5xx): the reference
    /// the organizer can give the operator, who finds it in the server's
    /// log. Not in the body: the client takes it from the response's
    /// `x-request-id` header, which every response carries.
    #[serde(skip)]
    pub request_id: Option<String>,
    /// For an add refused for its rows (`rows`): each row's problem, so the
    /// browser can show them where they are.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<crate::RowProblem>,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            request_id: None,
            rows: Vec::new(),
        }
    }

    /// This error, with the rows of an add that refused it.
    pub fn with_rows(self, rows: Vec<crate::RowProblem>) -> Self {
        Self { rows, ..self }
    }

    /// This error, for the request `id`.
    pub fn with_request_id(self, id: impl Into<String>) -> Self {
        Self {
            request_id: Some(id.into()),
            ..self
        }
    }
}

/// The error codes that aren't a refusal's: how a call failed before, or
/// instead of, reaching the raffle's rules.
pub mod codes {
    /// No session.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// No admin function by that name, or no such raffle, prize or entrant
    /// of the caller's.
    pub const NOT_FOUND: &str = "not_found";
    /// The arguments don't decode.
    pub const INVALID_ARGUMENTS: &str = "invalid_arguments";
    /// The request is larger than the admin API takes (512 KB).
    pub const TOO_LARGE: &str = "too_large";
    /// The raffle engine can't be reached.
    pub const UNAVAILABLE: &str = "unavailable";
    /// The server failed; the details are in its log.
    pub const INTERNAL: &str = "internal";
    /// The browser's client couldn't reach the server, or couldn't read
    /// what it sent back (an older page under a newer server).
    pub const CLIENT: &str = "client";
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
        fn $name:ident($($(#[$arg_meta:meta])* $arg:ident: $ty:ty),* $(,)?) -> $ret:ty;
    )*) => {
        /// The server's side of the admin API: every function, for the
        /// signed-in `caller` (the owner key raffles belong to, and the user).
        pub trait Admin: Sync {
            type Error;

            $(
                $(#[$meta])*
                fn $name(
                    &self,
                    caller: &$crate::Caller,
                    $($arg: $ty),*
                ) -> impl Future<Output = Result<$ret, Self::Error>> + Send;
            )*
        }

        /// Runs the admin function `name` for `caller` with JSON-encoded `args`
        /// (an empty body counts as `{}`) and returns its JSON-encoded result.
        pub async fn dispatch<A: Admin>(
            api: &A,
            caller: &$crate::Caller,
            name: &str,
            args: &[u8],
        ) -> Result<serde_json::Value, DispatchError<A::Error>> {
            let args = if args.is_empty() { b"{}" } else { args };
            match name {
                $(
                    stringify!($name) => {
                        #[derive(Deserialize)]
                        struct Args { $($(#[$arg_meta])* $arg: $ty),* }
                        let Args { $($arg),* } = serde_json::from_slice(args)
                            .map_err(|e| DispatchError::Arguments(e.to_string()))?;
                        let result = api.$name(caller, $($arg),*).await.map_err(DispatchError::App)?;
                        serde_json::to_value(result).map_err(DispatchError::Result)
                    }
                )*
                _ => Err(DispatchError::Unknown),
            }
        }
    };
}

admin_functions!(server_side);
