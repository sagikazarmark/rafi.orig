//! The public pages, rendered here: the participant page ([`page`]), the
//! raffle rules ([`rules`]), the privacy notice ([`privacy`]) and the
//! installation's documents ([`legal`]). They take plain inputs (no
//! database rows, no requests), and link the web app's stylesheet at
//! [`style::STYLESHEET_URL`].

mod participant;
mod style;
mod text;

pub use participant::{Entrant, Notice, Page, Raffle, not_found, page};
pub use text::{
    ErasureDates, Operator, Organizer, Privacy, PrizeLine, Rules, legal, markdown, privacy, rules,
};
