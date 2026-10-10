//! What a phone sees live, shared by the raffle's Durable Object, the app
//! server and the participant WASM.
//!
//! - [`ParticipantView`]: the slim view pushed to phones, and [`Phase`], a
//!   lightweight mirror of the raffle state;
//! - [`RaffleLive`]: the partyline channel that carries it;
//! - [`screen()`]: which card a phone shows, as a pure function;
//! - with the `card` feature, `Card`: the Dioxus component that renders it,
//!   on the server (SSR) and in the WASM alike, with [`CardStrings`];
//! - [`altcha`]: rafi's proof-of-work for the join form;
//! - [`LiveTransport`]: how clients follow a raffle on this deployment;
//! - [`PageConfig`]: the participant page's embedded state;
//! - [`publish::decide`]: when a live channel publishes a participant view;
//! - with the `raffle` feature, `raffle::view`: the participant view of a
//!   raffle as drawn, for the servers that build views.
//!
//! This crate must stay light enough for the phone: without `raffle` it
//! doesn't depend on `rafi-raffle` (which pulls in drand and BLS), without
//! `card` not on Dioxus (so the engine hosts don't build a UI framework), and it
//! reads no clock and no randomness of its own, so it runs unchanged on
//! `wasm32-unknown-unknown`.

pub mod altcha;
mod card;
mod page;
pub mod publish;
#[cfg(feature = "raffle")]
pub mod raffle;
mod screen;
mod transport;
mod view;

pub use card::{CARD, CardStrings, WON};
#[cfg(feature = "card")]
pub use card::{Card, CardProps};
pub use page::PageConfig;
pub use screen::{Screen, screen};
pub use transport::{DEFAULT_POLL_MS, LiveTransport};
pub use view::{
    Current, DrawInput, DrawOutcome, HIDDEN_NAME, LastDraw, ParticipantView, Phase, Winner,
    public_name,
};

/// The raffle's live channel: phones follow the latest [`ParticipantView`].
///
/// Latest mode keeps only the newest event, so a phone that (re)connects gets
/// the current view and nothing older. Phones order views by
/// [`ParticipantView::v`], so they need no partyline cursor.
pub struct RaffleLive;

impl partyline::Channel for RaffleLive {
    const NAME: &'static str = "raffle";
    const MODE: partyline::Mode = partyline::Mode::Latest;
    type Event = ParticipantView;
}
