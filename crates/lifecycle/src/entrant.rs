//! Who an entrant is: the name screens show, and the email address that
//! makes them one entrant (the store refuses a second entry with the same
//! [`Email::key`]).

use serde::{Deserialize, Serialize};

use crate::RaffleState;

/// How an entrant got in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Joined on the participant page.
    #[serde(rename = "self")]
    SelfService,
    /// Added by the organizer.
    Manual,
    /// Made up to try a raffle out (test mode only: [`fakes`]).
    Fake,
}

impl Source {
    /// Every source.
    pub const ALL: [Self; 3] = [Self::SelfService, Self::Manual, Self::Fake];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfService => "self",
            Self::Manual => "manual",
            Self::Fake => "fake",
        }
    }

    /// Whether an entrant from this source fixes the raffle's rules, event
    /// date and retention ([`rules_editable`](crate::settings::rules_editable)):
    /// what they entered under must stay what the raffle says. Someone who
    /// joined accepted them on the form; someone the organizer added accepted
    /// nothing (no consent is recorded for them), but was entered under them
    /// on the organizer's word, and may have been told them. Fakes are a
    /// rehearsal's and fix nothing. Exhaustive, so a new source has to decide.
    pub fn locks_settings(self) -> bool {
        match self {
            Self::SelfService | Self::Manual => true,
            Self::Fake => false,
        }
    }
}

/// The longest display name, in characters.
pub const MAX_NAME_CHARS: usize = 24;

/// The longest email address, in characters (RFC 5321's path limit).
pub const MAX_EMAIL_CHARS: usize = 254;

/// Words that are not accepted in display names (substring match, lowercase).
const BLOCKED: &[&str] = &[
    "fuck", "shit", "cunt", "nigger", "faggot", "hitler", "nazi", "bitch", "whore", "porn",
    "kurva", "geci", "fasz", "picsa", "buzi", "cigány", "köcsög", "kocsog", "baszd", "bazd",
    "anyád", "anyad",
];

/// Why a display name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// Empty, longer than [`MAX_NAME_CHARS`], with control or invisible
    /// characters, or without a single letter or digit.
    Invalid,
    /// Contains a blocked word.
    Blocked,
}

/// Whether a character has no business in a name shown on a screen: a
/// control character, or one of Unicode's *default ignorable* code points
/// (bidirectional controls like U+202E that reverse what follows, zero-width
/// joiners and spaces, soft hyphens, variation selectors, the Hangul fillers
/// U+115F, U+1160, U+3164 and U+FFA0 that render as blanks yet count as
/// letters, tag characters), or the Braille blank U+2800. Each lets a name
/// look like another ("Anna" and "Anna\u{200B}"), look empty, or slip a
/// blocked word past a substring check. The ranges are Unicode's
/// `Default_Ignorable_Code_Point` property, which is stable, written out so
/// no table crate is needed where this compiles (the Worker, the admin's
/// WebAssembly).
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// Whether `text` could be a name on a screen: no control or invisible
/// characters ([`is_invisible`]), and at least one letter or digit, so it
/// can't be blank and can't be another name with something unseen added.
pub fn legible(text: &str) -> bool {
    !text.chars().any(|c| c.is_control() || is_invisible(c))
        && text.chars().any(char::is_alphanumeric)
}

/// The display name `input` stands for, with its whitespace collapsed.
pub fn display_name(input: &str) -> Result<String, NameError> {
    let name = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let len = name.chars().count();
    if len == 0 || len > MAX_NAME_CHARS || !legible(&name) {
        return Err(NameError::Invalid);
    }
    let lower = name.to_lowercase();
    if BLOCKED.iter().any(|w| lower.contains(w)) {
        return Err(NameError::Blocked);
    }
    Ok(name)
}

/// The first word of a generated name ([`generated_name`]): friendly, so
/// none reads as an insult on the projector.
const ADJECTIVES: &[&str] = &[
    "Agile", "Amber", "Astute", "Bold", "Brave", "Breezy", "Bright", "Brisk", "Bubbly", "Calm",
    "Candid", "Cheerful", "Chipper", "Clever", "Cosmic", "Cozy", "Crafty", "Curious", "Dapper",
    "Daring", "Dazzling", "Eager", "Earnest", "Electric", "Fearless", "Fluffy", "Friendly",
    "Frosty", "Gentle", "Giddy", "Gleaming", "Golden", "Graceful", "Grand", "Happy", "Hardy",
    "Hearty", "Humble", "Jazzy", "Jolly", "Jovial", "Keen", "Kind", "Lively", "Lucky", "Lunar",
    "Magic", "Mellow", "Merry", "Mighty", "Misty", "Modest", "Nifty", "Nimble", "Noble", "Peppy",
    "Perky", "Placid", "Plucky", "Polite", "Proud", "Quick", "Quiet", "Quirky", "Radiant", "Rapid",
    "Rosy", "Royal", "Rustic", "Shiny", "Silent", "Silver", "Sleek", "Smart", "Snappy", "Snug",
    "Solar", "Sparkly", "Speedy", "Spry", "Steady", "Stellar", "Sunny", "Swift", "Tidy", "Trusty",
    "Upbeat", "Valiant", "Velvet", "Vivid", "Warm", "Wise", "Witty", "Zany", "Zesty", "Zippy",
];

/// The second word of a generated name ([`generated_name`]).
const ANIMALS: &[&str] = &[
    "Aardvark",
    "Alpaca",
    "Antelope",
    "Armadillo",
    "Badger",
    "Beaver",
    "Bison",
    "Bobcat",
    "Buffalo",
    "Camel",
    "Capybara",
    "Caribou",
    "Cheetah",
    "Chinchilla",
    "Cougar",
    "Coyote",
    "Crane",
    "Dingo",
    "Dolphin",
    "Dormouse",
    "Eagle",
    "Echidna",
    "Elephant",
    "Falcon",
    "Ferret",
    "Finch",
    "Flamingo",
    "Fox",
    "Gazelle",
    "Gecko",
    "Gerbil",
    "Giraffe",
    "Gopher",
    "Hamster",
    "Hedgehog",
    "Heron",
    "Hippo",
    "Ibex",
    "Iguana",
    "Impala",
    "Jaguar",
    "Kangaroo",
    "Kingfisher",
    "Kiwi",
    "Koala",
    "Lemur",
    "Leopard",
    "Llama",
    "Lobster",
    "Lynx",
    "Macaw",
    "Magpie",
    "Manatee",
    "Marmot",
    "Meerkat",
    "Mongoose",
    "Moose",
    "Narwhal",
    "Newt",
    "Ocelot",
    "Octopus",
    "Okapi",
    "Orca",
    "Osprey",
    "Otter",
    "Owl",
    "Panda",
    "Pangolin",
    "Panther",
    "Parrot",
    "Pelican",
    "Penguin",
    "Platypus",
    "Puffin",
    "Puma",
    "Quail",
    "Quokka",
    "Rabbit",
    "Raccoon",
    "Raven",
    "Reindeer",
    "Robin",
    "Salamander",
    "Seahorse",
    "Seal",
    "Sparrow",
    "Squirrel",
    "Starling",
    "Stork",
    "Swan",
    "Tapir",
    "Tiger",
    "Toucan",
    "Turtle",
    "Walrus",
    "Wombat",
    "Yak",
    "Zebra",
];

/// How many different names [`generated_name`] makes.
pub const GENERATED_NAMES: u64 = (ADJECTIVES.len() * ANIMALS.len()) as u64;

/// Generated name number `n` (modulo [`GENERATED_NAMES`]): an adjective and
/// an animal, like "Brave Otter". For entrants who leave their name blank,
/// the join form's suggestions, and fake entrants. Every one is a valid
/// [`display_name`].
pub fn generated_name(n: u64) -> String {
    let n = (n % GENERATED_NAMES) as usize;
    format!(
        "{} {}",
        ADJECTIVES[n / ANIMALS.len()],
        ANIMALS[n % ANIMALS.len()]
    )
}

/// An entrant's email address: as given, and the key that identifies the
/// entrant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    /// The address as given, trimmed: where mail goes.
    pub raw: String,
    /// The normalized address ([`Email::parse`]): one entry per key.
    pub key: String,
}

impl Email {
    /// Parses an address, or `None` if it is obviously malformed, longer
    /// than [`MAX_EMAIL_CHARS`], or has whitespace, control or invisible
    /// characters ([`is_invisible`]) in it.
    ///
    /// The unseen ones are refused as in a display name: an address with a
    /// zero-width space is another key for the same-looking mailbox (a second
    /// entry), a bidirectional control makes it read differently in the admin
    /// and the winners' export, and neither is in any address mail reaches.
    ///
    /// The key is lowercased, drops a `+tag`, and for Gmail drops dots and
    /// maps `googlemail.com` to `gmail.com`: the spellings of one mailbox
    /// make one entrant.
    pub fn parse(input: &str) -> Option<Self> {
        let raw = input.trim();
        if raw.chars().count() > MAX_EMAIL_CHARS
            || raw
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || is_invisible(c))
        {
            return None;
        }
        Some(Self {
            key: key(raw)?,
            raw: raw.to_owned(),
        })
    }
}

fn key(email: &str) -> Option<String> {
    let email = email.to_lowercase();
    let (local, domain) = email.rsplit_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return None;
    }
    if local.contains('@') {
        return None;
    }
    let local = local.split('+').next().unwrap_or(local);
    if local.is_empty() {
        return None;
    }
    let (local, domain) = match domain {
        "gmail.com" | "googlemail.com" => (local.replace('.', ""), "gmail.com"),
        _ => (local.to_owned(), domain),
    };
    Some(format!("{local}@{domain}"))
}

/// Why someone can't join on their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRefusal {
    /// Registration isn't open.
    Closed,
    /// They didn't accept the rules.
    Rules,
}

/// Whether someone may join a raffle in `state` on their own, having
/// accepted the rules or not, before or past its erasure date
/// (`past_erase_date`). Past the date the raffle is closed to them: the form
/// would promise erasure on a day gone by, and the raffle expires within the
/// hour (the organizer can still freeze it meanwhile, which keeps the list).
/// Their name and email are checked on their own ([`display_name`],
/// [`Email::parse`]).
pub fn may_join(
    state: RaffleState,
    past_erase_date: bool,
    accepted_rules: bool,
) -> Result<(), JoinRefusal> {
    if !state.joinable() || past_erase_date {
        return Err(JoinRefusal::Closed);
    }
    if !accepted_rules {
        return Err(JoinRefusal::Rules);
    }
    Ok(())
}

/// The most fake entrants added at once.
pub const MAX_FAKES: u32 = 500;

/// Why fake entrants can't be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FakesRefusal {
    /// Only a raffle in test mode takes fake entrants.
    NotTestMode,
    /// The entrant list is frozen.
    Frozen,
}

/// How many of `requested` fake entrants (rehearsal entrants) a raffle in
/// `state` takes: only in test mode, while its entrants can change, and at
/// most [`MAX_FAKES`] at once.
pub fn fakes(test_mode: bool, state: RaffleState, requested: u32) -> Result<u32, FakesRefusal> {
    if !test_mode {
        return Err(FakesRefusal::NotTestMode);
    }
    if !state.entrants_editable() {
        return Err(FakesRefusal::Frozen);
    }
    Ok(requested.min(MAX_FAKES))
}

/// Where an organizer's add comes from: the one-person form, or a list. Both
/// add people the organizer vouches for ([`Source::Manual`]) through the same
/// batch; they differ in when the raffle takes them ([`Origin::admits`]) and
/// in how the names reach the projector.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Not a display name ([`NameError::Invalid`]).
    Name,
    /// A display name with a blocked word ([`NameError::Blocked`]).
    BlockedName,
    /// Not an email address ([`Email::parse`]).
    Email,
    /// Not a registration ID ([`list_ref`]).
    ListRef,
    /// An earlier row of the same add has this email ([`Email::key`]).
    DuplicateEmail,
    /// An earlier row of the same add has this registration ID.
    DuplicateListRef,
}

/// A row of an add, checked: what the store is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedRow {
    pub name: String,
    pub email: Option<Email>,
    pub list_ref: Option<String>,
}

/// One row of an add, as the organizer sent it: a name, an email and a
/// registration ID, either blank for none.
pub fn added_row(
    name: &str,
    email: Option<&str>,
    list_ref: Option<&str>,
) -> Result<AddedRow, RowError> {
    let name = display_name(name).map_err(|e| match e {
        NameError::Invalid => RowError::Name,
        NameError::Blocked => RowError::BlockedName,
    })?;
    let email = match email.map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => Some(Email::parse(e).ok_or(RowError::Email)?),
        None => None,
    };
    let list_ref = match list_ref.filter(|r| !r.trim().is_empty()) {
        Some(r) => Some(self::list_ref(r).ok_or(RowError::ListRef)?),
        None => None,
    };
    Ok(AddedRow {
        name,
        email,
        list_ref,
    })
}

/// The rows of an add, checked ([`added_row`]), or every row's problem by
/// its index: a row whose email key or registration ID an earlier row has
/// is a duplicate, since one add can't say which of the two is meant.
pub fn added_rows<'a>(
    rows: impl IntoIterator<Item = (&'a str, Option<&'a str>, Option<&'a str>)>,
) -> Result<Vec<AddedRow>, Vec<(usize, RowError)>> {
    let mut emails = std::collections::HashSet::new();
    let mut refs = std::collections::HashSet::new();
    let mut checked = Vec::new();
    let mut problems = Vec::new();
    for (i, (name, email, list_ref)) in rows.into_iter().enumerate() {
        let row = added_row(name, email, list_ref).and_then(|row| {
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

/// How an entrant is removed ([`RaffleState::removal`](crate::RaffleState::removal)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Removal {
    /// Gone entirely: the entrant list isn't frozen yet.
    Delete,
    /// Their personal data is erased; their anonymous ID stays in the
    /// published list, so the draw stays verifiable.
    Erase,
}

#[cfg(test)]
mod tests {
    use super::*;
    use RaffleState as S;

    #[test]
    fn joining() {
        assert_eq!(may_join(S::Open, false, true), Ok(()));
        assert_eq!(may_join(S::Open, false, false), Err(JoinRefusal::Rules));
        assert_eq!(
            may_join(S::Open, true, true),
            Err(JoinRefusal::Closed),
            "past the erasure date, registration is closed"
        );
        for s in S::ALL.into_iter().filter(|s| *s != S::Open) {
            assert_eq!(may_join(s, false, true), Err(JoinRefusal::Closed), "{s:?}");
        }
    }

    #[test]
    fn fake_entrants() {
        assert_eq!(fakes(true, S::Open, 3), Ok(3));
        assert_eq!(fakes(true, S::Draft, MAX_FAKES + 1), Ok(MAX_FAKES));
        assert_eq!(fakes(false, S::Open, 3), Err(FakesRefusal::NotTestMode));
        assert_eq!(fakes(true, S::Frozen, 3), Err(FakesRefusal::Frozen));
    }

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

    #[test]
    fn rows_of_an_add() {
        let row = added_row(" Anna  K. ", Some(" Anna@X.org "), Some(" 7 ")).unwrap();
        assert_eq!(row.name, "Anna K.");
        assert_eq!(row.email.map(|e| e.key).as_deref(), Some("anna@x.org"));
        assert_eq!(row.list_ref.as_deref(), Some("7"));
        let blank = added_row("Bence", Some("  "), Some("")).unwrap();
        assert_eq!((blank.email, blank.list_ref), (None, None), "blank is none");
        assert_eq!(added_row("", None, None), Err(RowError::Name));
        assert_eq!(added_row("Kurva", None, None), Err(RowError::BlockedName));
        assert_eq!(added_row("Anna", Some("anna"), None), Err(RowError::Email));
        assert_eq!(
            added_row("Anna", None, Some("a\u{200B}")),
            Err(RowError::ListRef)
        );

        assert_eq!(
            added_rows([
                ("Anna", Some("anna@x.org"), Some("1")),
                ("Bence", None, None),
                ("Csilla", None, None),
            ])
            .map(|rows| rows.len()),
            Ok(3),
            "people without an email or an ID are fine, twice too"
        );
        assert_eq!(
            added_rows([
                ("Anna", Some("anna@x.org"), Some("1")),
                ("", None, None),
                ("Anna 2", Some("ANNA+x@x.org"), None),
                ("Dora", None, Some("1")),
                ("Emil", Some("bad"), None),
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

    #[test]
    fn an_email_key_is_its_own_key() {
        for input in [
            " Foo@Example.COM ",
            "a.b+meetup@gmail.com",
            "x.y@googlemail.com",
            "a+b+c@example.org",
        ] {
            let once = key(input).unwrap();
            assert_eq!(key(&once).as_deref(), Some(once.as_str()), "{input}");
        }
    }

    fn key(input: &str) -> Option<String> {
        Email::parse(input).map(|e| e.key)
    }

    #[test]
    fn email_keys() {
        assert_eq!(key(" Foo@Example.COM ").as_deref(), Some("foo@example.com"));
        assert_eq!(key("a.b+meetup@gmail.com").as_deref(), Some("ab@gmail.com"));
        assert_eq!(key("a.b@googlemail.com").as_deref(), Some("ab@gmail.com"));
        assert_eq!(key("a.b+x@example.com").as_deref(), Some("a.b@example.com"));
        assert_eq!(Email::parse(" A@x.com ").unwrap().raw, "A@x.com");
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "foo",
            "@x.com",
            "foo@",
            "foo@bar",
            "a b@x.com",
            "+x@x.com",
            "a@.com",
        ] {
            assert_eq!(key(bad), None, "{bad}");
        }
        let long = format!("{}@example.com", "a".repeat(MAX_EMAIL_CHARS));
        assert_eq!(key(&long), None, "too long");
    }

    /// Nothing unseen in an address, as in a display name: a zero-width
    /// space would make a second key for a mailbox that looks the same, a
    /// bidirectional control would make it read differently.
    #[test]
    fn rejects_control_and_invisible_characters() {
        for bad in [
            "a\tb@x.com",
            "a\u{0}b@x.com",
            "a\u{7}b@x.com",
            "ab@x.com\u{7f}",
            "a\u{85}b@x.com",
            "anna\u{200B}@x.com",
            "anna@x\u{200D}.com",
            "\u{202E}anna@x.com",
            "anna@x.com\u{2066}",
            "an\u{00AD}na@x.com",
            "anna\u{FEFF}@x.com",
            "anna@x.com\u{E0041}",
            "a\u{3164}@x.com",
            "a\u{00A0}b@x.com",
        ] {
            assert_eq!(Email::parse(bad), None, "{bad:?}");
        }
        // Around the address it is trimmed as before; letters beyond ASCII
        // are fine.
        assert_eq!(key("\tanna@x.com\n").as_deref(), Some("anna@x.com"));
        assert_eq!(key("Árvíz@példa.hu").as_deref(), Some("árvíz@példa.hu"));
    }

    #[test]
    fn what_locks_the_settings() {
        assert!(Source::SelfService.locks_settings());
        assert!(
            Source::Manual.locks_settings(),
            "accepted nothing, still entered under them"
        );
        assert!(!Source::Fake.locks_settings(), "a rehearsal's");
    }

    #[test]
    fn display_names() {
        assert_eq!(display_name("  Anna   K. ").as_deref(), Ok("Anna K."));
        assert_eq!(display_name(""), Err(NameError::Invalid));
        assert_eq!(
            display_name(&"x".repeat(MAX_NAME_CHARS + 1)),
            Err(NameError::Invalid)
        );
        assert_eq!(display_name("a\u{7}b"), Err(NameError::Invalid));
        assert_eq!(display_name("Kurva Anna"), Err(NameError::Blocked));
        // Nothing unseen: a zero-width space that makes a second "Anna", a
        // right-to-left override that reverses the line, a soft hyphen, the
        // Hangul filler that renders blank yet is a letter, a Braille blank;
        // and nothing without a letter or digit in it.
        for invisible in [
            "Anna\u{200B}",
            "\u{202E}Anna",
            "An\u{00AD}na",
            "\u{3164}",
            "\u{3164}\u{3164}",
            "\u{2800}",
            "Anna\u{FE0F}",
            "fu\u{200B}ck",
        ] {
            assert_eq!(
                display_name(invisible),
                Err(NameError::Invalid),
                "{invisible:?}"
            );
        }
        for blank in ["★ ★ ★", "---", "..."] {
            assert_eq!(display_name(blank), Err(NameError::Invalid), "{blank:?}");
        }
        assert_eq!(display_name("Bence 🎉").as_deref(), Ok("Bence 🎉"));
        assert_eq!(display_name("Árvíztűrő").as_deref(), Ok("Árvíztűrő"));
        assert_eq!(display_name("李雷").as_deref(), Ok("李雷"));
    }

    #[test]
    fn generated_names_are_display_names() {
        let names: std::collections::HashSet<String> =
            (0..GENERATED_NAMES).map(generated_name).collect();
        assert_eq!(names.len() as u64, GENERATED_NAMES, "no two alike");
        const { assert!(GENERATED_NAMES >= 9_000, "plenty for a full raffle") };
        for name in &names {
            assert_eq!(display_name(name).as_ref(), Ok(name), "{name}");
        }
        assert_eq!(generated_name(GENERATED_NAMES), generated_name(0));
        assert_eq!(generated_name(0), "Agile Aardvark");
    }
}
