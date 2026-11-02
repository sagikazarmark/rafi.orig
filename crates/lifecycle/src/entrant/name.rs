/// The longest display name someone gives themselves when they join, in
/// characters ([`display_name`]). It keeps a joiner from filling the
/// projector: anyone with the QR code can type anything there.
pub const MAX_NAME_CHARS: usize = 24;

/// The longest display name the organizer gives, in characters
/// ([`added_name`]): the people they add (one or a list) and any entrant
/// they rename. A registration list holds full names, often longer than a
/// joiner's limit, and a name shortened in the database would be less
/// recognizable to the winner and lost for good; the organizer's own list
/// is no troll's, and the screens fit long names instead.
pub const MAX_ADDED_NAME_CHARS: usize = 64;

/// Words that are not accepted in display names (substring match, lowercase).
const BLOCKED: &[&str] = &[
    "fuck", "shit", "cunt", "nigger", "faggot", "hitler", "nazi", "bitch", "whore", "porn",
    "kurva", "geci", "fasz", "picsa", "buzi", "cigány", "köcsög", "kocsog", "baszd", "bazd",
    "anyád", "anyad",
];

/// Why a display name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    /// Empty, longer than its limit ([`MAX_NAME_CHARS`] for a joiner,
    /// [`MAX_ADDED_NAME_CHARS`] for the organizer), with control or
    /// invisible characters, or without a single letter or digit.
    Invalid,
    /// Contains a blocked word, and is otherwise fine: only this one the
    /// organizer can approve ([`added_name_approved`]), since the
    /// blocklist's substring matching catches real names too.
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

/// The display name `input` stands for, as someone joining gives it: in
/// Unicode's composed form (NFC), with its whitespace collapsed, at most
/// [`MAX_NAME_CHARS`] characters, legible ([`legible`]) and without a
/// blocked word.
///
/// Composed first, so the characters counted and the names compared are
/// the ones a reader sees: "Á" typed as `A` and a combining acute (as some
/// keyboards and exports write it) is one character, and the same name as
/// the precomposed "Á", not a second, longer spelling of it.
pub fn display_name(input: &str) -> Result<String, NameError> {
    name_within(input, MAX_NAME_CHARS, true)
}

/// The display name `input` stands for, as the organizer gives it (adding
/// someone, or renaming any entrant): at most [`MAX_ADDED_NAME_CHARS`]
/// characters, every other rule [`display_name`]'s. The longer limit is the
/// only difference: the blocklist and the invisible characters guard the
/// screen against mistakes and pasted junk as much as against trolls.
pub fn added_name(input: &str) -> Result<String, NameError> {
    name_within(input, MAX_ADDED_NAME_CHARS, true)
}

/// [`added_name`] for a name the organizer approved: one the blocklist
/// refuses ([`NameError::Blocked`]) yet is someone's real name. Its
/// substring matching can't tell "Nazir", "Buzinkay" or "Shitara" from what
/// it is there for, and the organizer can: the approval lifts the blocklist
/// and nothing else. The length, the legibility and the invisible
/// characters still hold, since they guard the screen against what no
/// approval makes right (a name that looks like another, or like nothing).
/// Never [`NameError::Blocked`].
pub fn added_name_approved(input: &str) -> Result<String, NameError> {
    name_within(input, MAX_ADDED_NAME_CHARS, false)
}

/// `input` in Unicode's composed form (NFC), with its whitespace collapsed
/// to single spaces and trimmed: the spelling every display name is
/// compared and counted in ([`display_name`]).
pub(crate) fn tidy(input: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    input
        .nfc()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`display_name`]'s rules, with at most `max` characters, and the
/// blocklist if `blocklist`.
fn name_within(input: &str, max: usize, blocklist: bool) -> Result<String, NameError> {
    let name = tidy(input);
    let len = name.chars().count();
    if len == 0 || len > max || !legible(&name) {
        return Err(NameError::Invalid);
    }
    let lower = name.to_lowercase();
    if blocklist && BLOCKED.iter().any(|w| lower.contains(w)) {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A joiner gives up to 24 characters, the organizer up to 64, by the
    /// same rules otherwise; counted after whitespace is collapsed.
    #[test]
    fn name_lengths() {
        let joiner = "é".repeat(MAX_NAME_CHARS);
        assert_eq!(display_name(&joiner).as_ref(), Ok(&joiner), "24 is fine");
        assert_eq!(
            display_name(&format!("{joiner}x")),
            Err(NameError::Invalid),
            "25 isn't, for a joiner"
        );
        assert_eq!(
            added_name(&format!("{joiner}x")).map(|n| n.chars().count()),
            Ok(MAX_NAME_CHARS + 1),
            "it is for the organizer"
        );

        // A full name of 64 characters, as a registration list has it.
        let full = "Maria Fernanda de los Ángeles García-López y Hernández Rodríguez";
        assert_eq!(full.chars().count(), MAX_ADDED_NAME_CHARS);
        assert_eq!(added_name(full).as_deref(), Ok(full));
        assert_eq!(
            added_name(&format!("  {}  ", full.replace(' ', "   "))).as_deref(),
            Ok(full),
            "whitespace collapses before counting"
        );
        assert_eq!(added_name(&format!("{full}s")), Err(NameError::Invalid));
        assert_eq!(display_name(full), Err(NameError::Invalid));

        // Every other rule holds at any length.
        assert_eq!(added_name(""), Err(NameError::Invalid));
        assert_eq!(added_name("Anna\u{200B}"), Err(NameError::Invalid));
        assert_eq!(added_name("★ ★ ★"), Err(NameError::Invalid));
        assert_eq!(added_name("Kurva Anna"), Err(NameError::Blocked));
    }

    /// Names are composed (NFC): a decomposed "Á" is one character, the
    /// same name as the precomposed one, for joiners and the organizer.
    #[test]
    fn names_are_composed() {
        let decomposed = "A\u{301}rpa\u{301}d";
        assert_eq!(decomposed.chars().count(), 7);
        assert_eq!(display_name(decomposed).as_deref(), Ok("Árpád"));
        assert_eq!(added_name(decomposed).as_deref(), Ok("Árpád"));
        // 24 letters with a combining accent each: 48 code points, 24
        // characters once composed.
        let accented = "e\u{301}".repeat(MAX_NAME_CHARS);
        assert_eq!(
            display_name(&accented).map(|n| n.chars().count()),
            Ok(MAX_NAME_CHARS)
        );
        // A combining mark with nothing to compose with stays, and counts.
        assert_eq!(
            display_name("Zoe\u{0308}\u{0323}").map(|n| n.chars().count()),
            Ok(4),
            "an e with two marks: one composes with it, the other stays"
        );
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
