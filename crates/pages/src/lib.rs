//! Rafi's public pages, which the server renders: the participant page
//! ([`participant::page`]), the raffle rules ([`rules`]) and the privacy
//! notice ([`privacy`]). They take plain inputs (no database rows), and link
//! the web app's stylesheet at [`style::STYLESHEET_URL`].

pub mod participant;
pub mod style;
mod text;

pub use text::{Organizer, PrizeLine, Rules, privacy, rules};
