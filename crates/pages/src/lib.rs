//! Rafi's public pages, which the server renders: the participant page
//! ([`participant::page`]), the raffle rules ([`rules`]) and the privacy
//! notice ([`privacy`]). They take plain inputs (no database rows), so the
//! web app, which shares their text, icons and stylesheet, builds this
//! crate too:
//!
//! - [`i18n::Lang`]: the public pages' text in English and Hungarian (the
//!   projector's too);
//! - [`icons`]: the inline SVG icons;
//! - [`style::STYLESHEET_URL`]: where both link the compiled stylesheet.

pub mod i18n;
pub mod icons;
pub mod participant;
pub mod style;
mod text;

pub use text::{Organizer, PrizeLine, Rules, privacy, rules};
