//! What a phone sees live, shared by the raffle's Durable Object, the app
//! server and the attendee WASM.
//!
//! - [`ParticipantView`]: the slim view pushed to phones, and [`Phase`], a
//!   lightweight mirror of the raffle state;
//! - [`RaffleLive`]: the partyline channel that carries it;
//! - [`screen()`]: which card a phone shows, as a pure function;
//! - [`Card`]: the Dioxus component that renders it, on the server (SSR) and
//!   in the WASM alike, with [`CardStrings`];
//! - [`altcha`]: rafi's proof-of-work for the join form.
//!
//! This crate must stay light enough for the phone: it doesn't depend on
//! `rafi-core` (which pulls in drand and BLS), and it reads no clock and no
//! randomness of its own, so it runs unchanged on `wasm32-unknown-unknown`.

pub mod altcha;
mod card;
mod screen;
mod view;

pub use card::{CARD, Card, CardProps, CardStrings, WON};
pub use screen::{Screen, screen};
pub use view::{Current, DrawInput, DrawOutcome, LastDraw, ParticipantView, Phase, Winner};

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
