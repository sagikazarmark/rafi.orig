//! The public pages, rendered here: the participant page ([`page`]), the
//! raffle rules ([`rules`]), the privacy notice ([`privacy`]) and the
//! installation's documents ([`legal`]). They take plain inputs (no
//! database rows, no requests), and link the web app's stylesheet at
//! [`style::STYLESHEET_URL`].

mod participant;
mod style;
mod text;

pub(crate) use participant::inline_scripts;
pub use participant::{Entrant, Notice, Page, Raffle, not_found, page};
// Natively, `Dirs::check_participant` compares the directory served with it.
#[cfg(not(target_arch = "wasm32"))]
pub use participant::LOADER;
pub use text::{Operator, Organizer, Privacy, PrizeLine, Rules, legal, privacy, rules};
