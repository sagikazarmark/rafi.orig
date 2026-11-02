use serde::{Deserialize, Serialize};

use super::{Email, NameError, added_name, added_name_approved, is_invisible};
use crate::RaffleState;

/// Where an organizer's add comes from: the one-person form, or a list. Both
/// add people the organizer vouches for
/// ([`Source::Manual`](super::Source::Manual)) through the same batch; they
/// differ in when the raffle takes them ([`Origin::admits`]) and in how the
/// names reach the projector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// One person, typed in by the organizer.
    One,
    /// A list of people (pasted or uploaded), in its order.
    List,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::One => "one",
            Self::List => "list",
        }
    }

    /// Whether an add from here goes into a raffle in `state`: while the
    /// entrant list can change, and for a list once the raffle is published,
    /// since the paragraph the organizer sends its people links the rules
    /// and the privacy notice, which a draft doesn't serve yet.
    pub fn admits(self, state: RaffleState) -> bool {
        state.entrants_editable() && (self == Self::One || state.is_public())
    }

    /// Whether an add from here goes into a raffle in test mode. A list
    /// doesn't: a rehearsal's draws are public, and it has fake entrants to
    /// rehearse with; real people from a list would be drawn in it.
    pub fn admits_test_mode(self) -> bool {
        self == Self::One
    }
}

/// The most rows one add takes. A longer list is added in parts, each all
/// or nothing: it bounds the request's size, the rows checked at once and
/// the statements of its batch.
pub const MAX_ADD_ROWS: usize = 2000;

/// The longest batch ID, in characters.
pub const MAX_BATCH_ID_CHARS: usize = 64;

/// Whether `id` can name an add: 1 to [`MAX_BATCH_ID_CHARS`] ASCII letters,
/// digits, `-` or `_`. The browser makes it at random for each add and sends
/// it again on a retry, so a retried add is recognized and undone by it; it
/// is written to the audit log, so it can't carry anything but an ID.
pub fn batch_id(id: &str) -> bool {
    (1..=MAX_BATCH_ID_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The longest registration ID, in characters.
pub const MAX_LIST_REF_CHARS: usize = 64;

/// The registration ID `input` stands for (a row's ID in the organizer's
/// registration system, which matches it on a later add), trimmed; `None`
/// if it is empty, longer than [`MAX_LIST_REF_CHARS`], or has control or
/// invisible characters ([`is_invisible`]): an unseen character would make
/// a second ID that looks like the first.
pub fn list_ref(input: &str) -> Option<String> {
    let id = input.trim();
    let len = id.chars().count();
    ((1..=MAX_LIST_REF_CHARS).contains(&len)
        && !id.chars().any(|c| c.is_control() || is_invisible(c)))
    .then(|| id.to_owned())
}

/// Why the raffle doesn't take an add now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddRefusal {
    /// The entrant list is frozen.
    Frozen,
    /// The erasure date has passed: the raffle expires within the hour, and
    /// its entrants with it.
    ErasureDatePassed,
    /// A list goes into a published raffle only ([`Origin::admits`]).
    Unpublished,
    /// A list doesn't go into a raffle in test mode
    /// ([`Origin::admits_test_mode`]).
    TestMode,
}

/// Whether a raffle in `state`, in test mode or not, before or past its
/// erasure date, takes an add from `origin`. The store checks the same in
/// the add's own SQL, so nothing lands after a freeze in between.
pub fn may_add(
    origin: Origin,
    state: RaffleState,
    test_mode: bool,
    past_erase_date: bool,
) -> Result<(), AddRefusal> {
    if !state.entrants_editable() {
        return Err(AddRefusal::Frozen);
    }
    if past_erase_date {
        return Err(AddRefusal::ErasureDatePassed);
    }
    if !origin.admits(state) {
        return Err(AddRefusal::Unpublished);
    }
    if test_mode && !origin.admits_test_mode() {
        return Err(AddRefusal::TestMode);
    }
    Ok(())
}

/// What is wrong with a row of an add. An add with any is refused whole,
/// with each row's problem, so the organizer fixes them and sends it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowError {
    /// Not a display name the organizer may give ([`added_name`],
    /// [`NameError::Invalid`]).
    Name,
    /// A display name with a blocked word ([`NameError::Blocked`]), not
    /// approved: the organizer can approve it ([`RowInput::approved`]) if
    /// it is someone's real name.
    BlockedName,
    /// Not an email address ([`Email::parse`]).
    Email,
    /// Not a registration ID ([`list_ref`]).
    ListRef,
    /// An earlier row of the same add has this email ([`Email::key`]).
    DuplicateEmail,
    /// An earlier row of the same add has this registration ID.
    DuplicateListRef,
    /// Someone in the raffle is already shown with this display name
    /// ([`name_key`](crate::names::name_key)): two people on the screen
    /// with one name couldn't tell who won
    /// ([`name_clashes`](crate::names::name_clashes)).
    NameTaken,
    /// An earlier row of the same add, one that is added, is shown with
    /// this display name.
    DuplicateName,
}

/// One row of an add, as the organizer sent it: a name ([`added_name`]: up
/// to [`MAX_ADDED_NAME_CHARS`](super::MAX_ADDED_NAME_CHARS) characters), an
/// email and a registration ID, either blank for none, and whether the
/// organizer approved a name the blocklist refuses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RowInput<'a> {
    pub name: &'a str,
    pub email: Option<&'a str>,
    pub list_ref: Option<&'a str>,
    /// The organizer approved the name though the blocklist refuses it
    /// ([`added_name_approved`]): it lifts the blocklist only.
    pub approved: bool,
}

/// A row of an add, checked: what the store is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedRow {
    pub name: String,
    pub email: Option<Email>,
    pub list_ref: Option<String>,
    /// Its name is in only because the organizer approved it: the
    /// blocklist refuses it ([`RowInput::approved`]). An approval of a name
    /// the blocklist takes anyway approves nothing, and isn't one.
    pub approved: bool,
}

/// One row of an add, checked ([`RowInput`]).
pub fn added_row(row: RowInput<'_>) -> Result<AddedRow, RowError> {
    let (name, approved) = match added_name(row.name) {
        Ok(name) => (name, false),
        Err(NameError::Blocked) if row.approved => (
            added_name_approved(row.name).map_err(|_| RowError::Name)?,
            true,
        ),
        Err(NameError::Blocked) => return Err(RowError::BlockedName),
        Err(NameError::Invalid) => return Err(RowError::Name),
    };
    let email = match row.email.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => Some(Email::parse(e).ok_or(RowError::Email)?),
        None => None,
    };
    let list_ref = match row.list_ref.filter(|r| !r.trim().is_empty()) {
        Some(r) => Some(self::list_ref(r).ok_or(RowError::ListRef)?),
        None => None,
    };
    Ok(AddedRow {
        name,
        email,
        list_ref,
        approved,
    })
}

/// The rows of an add, checked ([`added_row`]), or every row's problem by
/// its index: a row whose email key or registration ID an earlier row has
/// is a duplicate, since one add can't say which of the two is meant.
/// Whether their display names are unique depends on who is in the raffle
/// and which rows it already has: [`name_clashes`](crate::names::name_clashes).
pub fn added_rows<'a>(
    rows: impl IntoIterator<Item = RowInput<'a>>,
) -> Result<Vec<AddedRow>, Vec<(usize, RowError)>> {
    let mut emails = std::collections::HashSet::new();
    let mut refs = std::collections::HashSet::new();
    let mut checked = Vec::new();
    let mut problems = Vec::new();
    for (i, input) in rows.into_iter().enumerate() {
        let row = added_row(input).and_then(|row| {
            if row
                .email
                .as_ref()
                .is_some_and(|e| !emails.insert(e.key.clone()))
            {
                return Err(RowError::DuplicateEmail);
            }
            if row
                .list_ref
                .as_ref()
                .is_some_and(|r| !refs.insert(r.clone()))
            {
                return Err(RowError::DuplicateListRef);
            }
            Ok(row)
        });
        match row {
            Ok(row) => checked.push(row),
            Err(e) => problems.push((i, e)),
        }
    }
    if problems.is_empty() {
        Ok(checked)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::super::MAX_ADDED_NAME_CHARS;
    use super::*;
    use RaffleState as S;

    #[test]
    fn who_takes_an_add() {
        use Origin::{List, One};
        for s in S::ALL {
            let editable = s.entrants_editable();
            assert_eq!(One.admits(s), editable, "{s:?}");
            assert_eq!(List.admits(s), editable && s != S::Draft, "{s:?}");
        }
        assert_eq!(may_add(One, S::Draft, true, false), Ok(()));
        assert_eq!(may_add(List, S::Open, false, false), Ok(()));
        assert_eq!(
            may_add(List, S::Draft, false, false),
            Err(AddRefusal::Unpublished)
        );
        assert_eq!(
            may_add(List, S::Ready, true, false),
            Err(AddRefusal::TestMode)
        );
        assert_eq!(
            may_add(One, S::Closed, false, true),
            Err(AddRefusal::ErasureDatePassed)
        );
        for frozen in [S::Frozen, S::Drawing, S::Finished, S::Purged] {
            for origin in [One, List] {
                assert_eq!(
                    may_add(origin, frozen, false, false),
                    Err(AddRefusal::Frozen),
                    "frozen first: {origin:?} {frozen:?}"
                );
            }
        }
    }

    #[test]
    fn batch_ids() {
        for good in [
            "a",
            "0f3c9a1e-77b2-4f43-9d3a-2c1b5e6f7a80",
            "A_b-9",
            &"x".repeat(64),
        ] {
            assert!(batch_id(good), "{good:?}");
        }
        for bad in ["", "a b", "a/b", "anna@x.org", "é", &"x".repeat(65)] {
            assert!(!batch_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn registration_ids() {
        assert_eq!(list_ref("  TCK-0042 ").as_deref(), Some("TCK-0042"));
        assert_eq!(list_ref("Order #12 / 3").as_deref(), Some("Order #12 / 3"));
        assert_eq!(
            list_ref(&"é".repeat(64)).map(|r| r.chars().count()),
            Some(64)
        );
        for bad in [
            "",
            "   ",
            "a\u{200B}b",
            "a\tb",
            "\u{202E}1",
            &"x".repeat(65),
        ] {
            assert_eq!(list_ref(bad), None, "{bad:?}");
        }
    }

    /// A row as the tests write it: name, email, registration ID.
    fn row<'a>(name: &'a str, email: Option<&'a str>, list_ref: Option<&'a str>) -> RowInput<'a> {
        RowInput {
            name,
            email,
            list_ref,
            approved: false,
        }
    }

    #[test]
    fn rows_of_an_add() {
        let added = added_row(row(" Anna  K. ", Some(" Anna@X.org "), Some(" 7 "))).unwrap();
        assert_eq!(added.name, "Anna K.");
        assert_eq!(added.email.map(|e| e.key).as_deref(), Some("anna@x.org"));
        assert_eq!(added.list_ref.as_deref(), Some("7"));
        assert!(!added.approved);
        let long = "x".repeat(MAX_ADDED_NAME_CHARS);
        assert_eq!(added_row(row(&long, None, None)).map(|r| r.name), Ok(long));
        assert_eq!(
            added_row(row(&"x".repeat(MAX_ADDED_NAME_CHARS + 1), None, None)),
            Err(RowError::Name),
            "the organizer's limit, not a joiner's"
        );
        let blank = added_row(row("Bence", Some("  "), Some(""))).unwrap();
        assert_eq!((blank.email, blank.list_ref), (None, None), "blank is none");
        assert_eq!(added_row(row("", None, None)), Err(RowError::Name));
        assert_eq!(
            added_row(row("Kurva", None, None)),
            Err(RowError::BlockedName)
        );
        assert_eq!(
            added_row(row("Anna", Some("anna"), None)),
            Err(RowError::Email)
        );
        assert_eq!(
            added_row(row("Anna", None, Some("a\u{200B}"))),
            Err(RowError::ListRef)
        );

        assert_eq!(
            added_rows([
                row("Anna", Some("anna@x.org"), Some("1")),
                row("Bence", None, None),
                row("Csilla", None, None),
            ])
            .map(|rows| rows.len()),
            Ok(3),
            "people without an email or an ID are fine, twice too"
        );
        assert_eq!(
            added_rows([
                row("Anna", Some("anna@x.org"), Some("1")),
                row("", None, None),
                row("Anna 2", Some("ANNA+x@x.org"), None),
                row("Dora", None, Some("1")),
                row("Emil", Some("bad"), None),
            ]),
            Err(vec![
                (1, RowError::Name),
                (2, RowError::DuplicateEmail),
                (3, RowError::DuplicateListRef),
                (4, RowError::Email),
            ]),
            "every row's problem, by index; the later of two is the duplicate"
        );
    }

    /// The blocklist's substring matching catches real names; the organizer
    /// approves them row by row, which lifts the blocklist and nothing else.
    #[test]
    fn approved_names() {
        let approved = |name| RowInput {
            name,
            approved: true,
            ..RowInput::default()
        };
        for real in ["Nazir Ahmed", "Buzinkay Gábor", "Shitara Ken"] {
            assert_eq!(
                added_row(row(real, None, None)),
                Err(RowError::BlockedName),
                "{real}: refused without the approval"
            );
            let added = added_row(approved(real)).unwrap();
            assert_eq!(added.name, real);
            assert!(added.approved, "{real}: in by the approval");
            assert_eq!(added_name(real), Err(NameError::Blocked));
            assert_eq!(added_name_approved(real).as_deref(), Ok(real));
        }
        let fine = added_row(approved("Anna Kovács")).unwrap();
        assert!(
            !fine.approved,
            "approving a name the blocklist takes approves nothing"
        );
        // Every other rule holds, approved or not.
        let long = format!("Nazir {}", "x".repeat(MAX_ADDED_NAME_CHARS));
        for bad in ["", "Nazir\u{200B}", "\u{202E}Nazir", "★ ★ ★", &long] {
            assert_eq!(added_row(approved(bad)), Err(RowError::Name), "{bad:?}");
            assert_eq!(added_name_approved(bad), Err(NameError::Invalid), "{bad:?}");
        }
        assert_eq!(
            added_rows([approved("Nazir"), row("Nazir Ali", None, None)]),
            Err(vec![(1, RowError::BlockedName)]),
            "an approval is the row's own"
        );
    }
}
