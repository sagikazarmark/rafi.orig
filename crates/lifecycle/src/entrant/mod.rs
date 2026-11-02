//! Who an entrant is: the name screens show, and the email address that
//! makes them one entrant (the store refuses a second entry with the same
//! [`Email::key`]).

mod add;
mod email;
mod fake;
mod join;
mod name;
mod removal;
mod source;

pub use add::{
    AddRefusal, AddedRow, MAX_ADD_ROWS, MAX_BATCH_ID_CHARS, MAX_LIST_REF_CHARS, Origin, RowError,
    RowInput, added_row, added_rows, batch_id, list_ref, may_add,
};
pub use email::{Email, MAX_EMAIL_CHARS};
pub use fake::{FakesRefusal, MAX_FAKES, fakes};
pub use join::{JoinRefusal, may_join};
pub(crate) use name::tidy;
pub use name::{
    GENERATED_NAMES, MAX_ADDED_NAME_CHARS, MAX_NAME_CHARS, NameError, added_name,
    added_name_approved, display_name, generated_name, is_invisible, legible,
};
pub use removal::{ObjectionRefusal, Removal, may_object, may_unobject, objected_may_be};
pub use source::Source;
