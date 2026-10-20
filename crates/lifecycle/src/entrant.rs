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

    /// Whether an entrant from this source accepted the raffle's rules:
    /// everyone but fakes (the organizer vouches for people added by hand).
    /// Once anyone has, the rules are fixed
    /// ([`rules_editable`](crate::settings::rules_editable)).
    pub fn accepted_rules(self) -> bool {
        !matches!(self, Self::Fake)
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
    /// Empty, longer than [`MAX_NAME_CHARS`], or with control characters.
    Invalid,
    /// Contains a blocked word.
    Blocked,
}

/// The display name `input` stands for, with its whitespace collapsed.
pub fn display_name(input: &str) -> Result<String, NameError> {
    let name = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let len = name.chars().count();
    if len == 0 || len > MAX_NAME_CHARS || name.chars().any(char::is_control) {
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
    /// Parses an address, or `None` if it is obviously malformed or longer
    /// than [`MAX_EMAIL_CHARS`].
    ///
    /// The key is lowercased, drops a `+tag`, and for Gmail drops dots and
    /// maps `googlemail.com` to `gmail.com`: the spellings of one mailbox
    /// make one entrant.
    pub fn parse(input: &str) -> Option<Self> {
        let raw = input.trim();
        if raw.chars().count() > MAX_EMAIL_CHARS {
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
    if email.chars().any(char::is_whitespace) || local.contains('@') {
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
/// accepted the rules or not. Their name and email are checked on their own
/// ([`display_name`], [`Email::parse`]).
pub fn may_join(state: RaffleState, accepted_rules: bool) -> Result<(), JoinRefusal> {
    if !state.joinable() {
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

/// How an entrant is removed ([`RaffleState::removal`](crate::RaffleState::removal)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        assert_eq!(may_join(S::Open, true), Ok(()));
        assert_eq!(may_join(S::Open, false), Err(JoinRefusal::Rules));
        for s in S::ALL.into_iter().filter(|s| *s != S::Open) {
            assert_eq!(may_join(s, true), Err(JoinRefusal::Closed), "{s:?}");
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
