//! Adding a list (the Add people dialog's list tab), without the UI: pure,
//! natively tested, built for the browser too.
//!
//! 1. **Reading** ([`parse()`]): pasted cells (a spreadsheet copies them
//!    tab-separated) or a file's bytes ([`decode`]: UTF-8, else an error
//!    that says to paste), within [limits](parse::MAX_INPUT_BYTES), the
//!    delimiter sniffed, RFC 4180 quotes.
//! 2. **Mapping** ([`detect`]): which row is the header and which column is
//!    the name (one, or first and last), the email and the registration ID,
//!    by the headers of the common exports, or by what the cells hold. The
//!    organizer corrects it.
//! 3. **The preview** ([`preview()`]): every row checked by the rules the
//!    server applies again ([`rafi_lifecycle::entrant`]), matched against
//!    the raffle's entrants and the list's earlier rows, given a unique
//!    display name ([`rafi_lifecycle::names`]); the organizer's decisions
//!    ([`Decisions`]) approve blocked names, keep group orders whole or
//!    not, and add people removed at their request anyway. Its
//!    [`rows`](Preview::rows) are the request.
//!
//! The list stays in the page's memory: nothing of it is stored or logged,
//! and only the mapped fields of the rows to add are sent.
//!
//! An add takes at most [`MAX_ADD_ROWS`](rafi_lifecycle::entrant::MAX_ADD_ROWS)
//! rows; a preview with more to add can't be sent ([`Preview::too_many`]),
//! and the organizer adds the list in parts, the people already in skipped
//! by the next one. Sending parts by itself would make one list several
//! adds, of which "Undo last add" takes back the last only, and a part
//! that fails after others went in a half-added list.
//!
//! **Batch IDs** ([`batch_id`]): an add is sent with a batch ID, which a
//! retry repeats so the server answers the first add instead of adding the
//! people twice. The ID goes with the rows sent: the same rows sent again
//! (after a network error) reuse it, other rows get a new one. Since the
//! people of an add that went in unseen would change the preview (they
//! are in now), the tab leaves that add's entrants out of the preview
//! until it knows how the add went.

mod columns;
mod parse;
mod preview;

#[cfg(test)]
mod tests;

pub use columns::{Mapping, NameColumns, column_labels, detect, detect_with_header, shorten};
pub use parse::{MAX_INPUT_BYTES, ParseError, Table, decode, parse};
pub use preview::{Decisions, Existing, MatchedBy, Preview, PreviewError, preview};

use rafi_api::AddRow;

/// An add sent and not known to have gone in or not: the rows and the
/// batch ID they went with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub batch_id: String,
    pub rows: Vec<AddRow>,
}

/// The batch ID to send `rows` with: the last attempt's if it sent the
/// same rows (a retry), else a `fresh` one.
pub fn batch_id(last: Option<&Attempt>, rows: &[AddRow], fresh: impl FnOnce() -> String) -> String {
    match last {
        Some(a) if a.rows == rows => a.batch_id.clone(),
        _ => fresh(),
    }
}
