//! How the names on an organizer's list become display names: the parts of
//! a name ([`NameParts`]), the way they are shown ([`NameFormat`]), the key
//! that says two display names are the same ([`name_key`]), the names a
//! list gets so nobody on the screen shares one ([`unique_names`]), and the
//! server's check that nobody does ([`name_clashes`]).
//!
//! The browser makes a list's names with these before it sends them; the
//! server only checks what it receives, never reformats it.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::entrant::{MAX_ADDED_NAME_CHARS, RowError, tidy};

/// `s` without its accents: decomposed (NFD), the combining marks dropped
/// ("Kovács Ősz" is "Kovacs Osz"). For matching and for slugs, never for a
/// name shown.
pub fn fold_accents(s: &str) -> String {
    use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
    s.nfd().filter(|c| !is_combining_mark(*c)).collect()
}

/// How the names on a list are shown: the organizer's choice for one add.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NameFormat {
    /// The full name, as the list has it: "Anna Kovács". The default: a
    /// winner recognizes it without a phone telling them, and the room sees
    /// the list is made of real people.
    #[default]
    Full,
    /// The given name and the family name's initial: "Anna K.", for a large
    /// event, or one that calls for less on the screen.
    FirstInitial,
}

impl NameFormat {
    /// Every format, the default first.
    pub const ALL: [Self; 2] = [Self::Full, Self::FirstInitial];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::FirstInitial => "first_initial",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "full" => Self::Full,
            "first_initial" => Self::FirstInitial,
            _ => return None,
        })
    }

    /// A name in this format, to show what it looks like.
    pub fn example(self) -> &'static str {
        match self {
            Self::Full => "Anna Kovács",
            Self::FirstInitial => "Anna K.",
        }
    }

    /// The format in the organizer's words, to follow "shown as": "your
    /// full name".
    pub fn described(self) -> &'static str {
        match self {
            Self::Full => "your full name",
            Self::FirstInitial => "your first name and the initial of your last name",
        }
    }
}

/// The most letters of the family name [`NameFormat::FirstInitial`] shows
/// to tell two people apart ("Anna K.", "Anna Ko.", "Anna Kov."), before
/// [`unique_names`] numbers them instead.
pub const MAX_INITIAL_LETTERS: usize = 3;

/// A person's name in its parts: the given name(s) and the family name,
/// either possibly empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct NameParts {
    pub given: String,
    pub family: String,
}

impl NameParts {
    pub fn new(given: &str, family: &str) -> Self {
        Self {
            given: given.to_owned(),
            family: family.to_owned(),
        }
    }

    /// The parts of a full name in one column: the family name is its last
    /// word, everything before it given names ("Anna Mária Kovács"); or, if
    /// the list writes `family_first` (Hungarian, Chinese, Japanese and
    /// Korean lists often do), its first word, everything after it given
    /// names ("Kovács Anna" is Anna Kovács). A single word is a given name.
    /// Family names of several words ("de la Cruz") can't be told apart
    /// from given names in one column: the list's first and last name
    /// columns can.
    pub fn split(full: &str, family_first: bool) -> Self {
        let name = tidy(full);
        let words: Vec<&str> = name.split(' ').filter(|w| !w.is_empty()).collect();
        match words.as_slice() {
            [] => Self::default(),
            [one] => Self::new(one, ""),
            [first, rest @ ..] if family_first => Self::new(&rest.join(" "), first),
            [rest @ .., last] => Self::new(&rest.join(" "), last),
        }
    }

    /// The display name these parts make in `format`: the full name, given
    /// names first, or the given names and the family name's initial ("Anna
    /// K."). Composed (NFC) and with its whitespace collapsed, as display
    /// names are; not checked ([`added_name`](crate::entrant::added_name)
    /// does). Without a family name, the given names alone; without given
    /// names, the family name whole, in either format: "K." alone would be
    /// nobody's name.
    pub fn display(&self, format: NameFormat) -> String {
        self.shown(format, 1)
    }

    /// [`display`](Self::display), with `letters` of the family name for
    /// [`NameFormat::FirstInitial`].
    fn shown(&self, format: NameFormat, letters: usize) -> String {
        let (given, family) = (tidy(&self.given), tidy(&self.family));
        let family = match format {
            NameFormat::Full => family,
            NameFormat::FirstInitial if given.is_empty() => family,
            NameFormat::FirstInitial => abbreviated(&family, letters),
        };
        tidy(&format!("{given} {family}"))
    }
}

/// The first `letters` letters of `family` and a full stop ("Ko."), or
/// `family` whole if that is all of it. A letter is a character with the
/// combining marks after it (an accent that doesn't compose), so none is
/// cut from its letter; what isn't a letter or a digit (an apostrophe, a
/// hyphen, a space) is kept between letters but not counted, so "O'Brien"
/// abbreviates to "O.", then "O'B.".
fn abbreviated(family: &str, letters: usize) -> String {
    use unicode_normalization::char::is_combining_mark;
    let mut counted = 0;
    let mut end = 0;
    for (i, c) in family.char_indices() {
        if is_combining_mark(c) {
            end = i + c.len_utf8();
            continue;
        }
        if counted == letters {
            break;
        }
        end = i + c.len_utf8();
        if c.is_alphanumeric() {
            counted += 1;
        }
    }
    if end >= family.len() {
        return family.to_owned();
    }
    let prefix =
        family[..end].trim_end_matches(|c: char| !c.is_alphanumeric() && !is_combining_mark(c));
    format!("{prefix}.")
}

/// The key that makes two display names the same name on the screen:
/// composed (NFC), whitespace collapsed, lowercase. "Anna K.", "anna  k."
/// and "Anna K." with a decomposed accent are one name. Display names are
/// unique by it within a raffle when the organizer adds people
/// ([`name_clashes`]); joiners and renames aren't held to it.
pub fn name_key(name: &str) -> String {
    tidy(name).to_lowercase()
}

/// The display names for the rows of a list, in `format`, each unique by
/// [`name_key`] among `taken` (the display names already in the raffle)
/// and among each other. What the browser sends; the server checks it
/// ([`name_clashes`]).
///
/// Names that would collide are told apart:
///
/// - with [`NameFormat::FirstInitial`], by more letters of the family name,
///   at most [`MAX_INITIAL_LETTERS`] ("Anna Kovács" and "Anna Kiss" are
///   "Anna Ko." and "Anna Ki."). Every new row of a colliding group
///   expands, so neither of two people keeps the shorter name as if it
///   were theirs alone; a group expands only while that tells someone apart
///   or moves them off a name already in the raffle;
/// - then, and with [`NameFormat::Full`], by a number: real namesakes are
///   "Anna Kovács" and "Anna Kovács (2)". The first new row of a group keeps
///   the name, unless someone in the raffle has it already.
///
/// Only new rows change: the people already in keep their names. A number
/// that would take a name over [`MAX_ADDED_NAME_CHARS`] shortens the name
/// before it by whole characters; a name already too long is left for the
/// length check to refuse.
pub fn unique_names<'a>(
    taken: impl IntoIterator<Item = &'a str>,
    rows: &[NameParts],
    format: NameFormat,
) -> Vec<String> {
    let taken: HashSet<String> = taken.into_iter().map(name_key).collect();
    let max_letters = match format {
        NameFormat::Full => 1,
        NameFormat::FirstInitial => MAX_INITIAL_LETTERS,
    };
    let mut letters = vec![1; rows.len()];
    let mut names: Vec<String> = rows.iter().map(|r| r.display(format)).collect();

    // More letters, group by group: as few more as tell someone apart, or
    // move them off a name in the raffle; none if no number of them does.
    loop {
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, name) in names.iter().enumerate() {
            groups.entry(name_key(name)).or_default().push(i);
        }
        let mut changed = false;
        for (key, members) in groups {
            let in_raffle = taken.contains(&key);
            if members.len() < 2 && !in_raffle {
                continue;
            }
            for more in 1..max_letters {
                let next: Vec<(usize, String)> = members
                    .iter()
                    .map(|&i| {
                        let l = (letters[i] + more).min(max_letters);
                        (l, rows[i].shown(format, l))
                    })
                    .collect();
                let keys: HashSet<String> = next.iter().map(|(_, n)| name_key(n)).collect();
                if keys.len() > 1 || (in_raffle && !keys.contains(&key)) {
                    for (&i, (l, name)) in members.iter().zip(next) {
                        changed |= name_key(&name) != key;
                        letters[i] = l;
                        names[i] = name;
                    }
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Then numbers. Every name that stays as it is is reserved first, so
    // no number lands on it: a unique one, and the first of a group that
    // isn't in the raffle yet.
    let keys: Vec<String> = names.iter().map(|n| name_key(n)).collect();
    let mut used = taken;
    let mut numbered = vec![false; rows.len()];
    for (i, key) in keys.iter().enumerate() {
        numbered[i] = !used.insert(key.clone());
    }
    // Where the search for a free number goes on, by name: the numbers
    // before it are taken, and stay so, so a list of many namesakes isn't
    // quadratic.
    let mut next: HashMap<String, usize> = HashMap::new();
    for (i, name) in names.iter_mut().enumerate() {
        if !numbered[i] {
            continue;
        }
        let from = next.get(name.as_str()).copied().unwrap_or(2);
        for n in from.. {
            let candidate = with_number(name, n);
            if used.insert(name_key(&candidate)) {
                next.insert(name.clone(), n + 1);
                *name = candidate;
                break;
            }
        }
    }
    names
}

/// `name` with the number `n` after it ("Anna Kovács (2)"), shortened by
/// whole characters, if need be, so the whole is at most
/// [`MAX_ADDED_NAME_CHARS`].
fn with_number(name: &str, n: usize) -> String {
    let suffix = format!(" ({n})");
    let len = name.chars().count();
    let room = MAX_ADDED_NAME_CHARS.saturating_sub(suffix.chars().count());
    if len <= MAX_ADDED_NAME_CHARS && len > room {
        let cut: String = name.chars().take(room).collect();
        format!("{}{suffix}", cut.trim_end())
    } else {
        format!("{name}{suffix}")
    }
}

/// A row's display name that someone else is already shown with
/// ([`name_clashes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameClash {
    /// The row, as the caller numbered it.
    pub row: usize,
    /// [`RowError::NameTaken`] or [`RowError::DuplicateName`].
    pub error: RowError,
    /// The name the other one is shown as, as they have it.
    pub shown_as: String,
}

/// The rows (by their index) whose display name, by [`name_key`], is
/// already someone's in the raffle (`taken`) or an earlier row's, with the
/// name they would share: the server's check that the names the organizer
/// adds are unique, which it refuses an add for. Only the rows that will be
/// added belong in `names`: one already in (by its email or registration
/// ID) is skipped and shares its name with itself.
pub fn name_clashes<'a>(
    taken: impl IntoIterator<Item = &'a str>,
    names: impl IntoIterator<Item = (usize, &'a str)>,
) -> Vec<NameClash> {
    let mut in_raffle: HashMap<String, &str> = HashMap::new();
    for name in taken {
        in_raffle.entry(name_key(name)).or_insert(name);
    }
    let mut earlier: HashMap<String, &str> = HashMap::new();
    let mut clashes = Vec::new();
    for (row, name) in names {
        let key = name_key(name);
        if let Some(other) = in_raffle.get(&key) {
            clashes.push(NameClash {
                row,
                error: RowError::NameTaken,
                shown_as: (*other).to_owned(),
            });
        } else if let Some(other) = earlier.get(&key) {
            clashes.push(NameClash {
                row,
                error: RowError::DuplicateName,
                shown_as: (*other).to_owned(),
            });
        } else {
            earlier.insert(key, name);
        }
    }
    clashes
}

#[cfg(test)]
mod tests {
    use super::*;
    use NameFormat::{FirstInitial, Full};

    #[test]
    fn formats_roundtrip() {
        for f in NameFormat::ALL {
            assert_eq!(NameFormat::parse(f.as_str()), Some(f));
            let json = serde_json::to_string(&f).unwrap();
            assert_eq!(
                json,
                format!("\"{}\"", f.as_str()),
                "serde and the audit agree"
            );
        }
        assert_eq!(NameFormat::default(), Full);
        assert_eq!(NameFormat::parse("initials"), None);
        for f in NameFormat::ALL {
            assert_eq!(
                NameParts::split(f.example(), false).display(f),
                f.example(),
                "the example is what the format makes"
            );
        }
    }

    #[test]
    fn splitting_a_full_name() {
        let split = |s, family_first| {
            let p = NameParts::split(s, family_first);
            (p.given, p.family)
        };
        let parts = |g: &str, f: &str| (g.to_owned(), f.to_owned());
        assert_eq!(split("Anna Kovács", false), parts("Anna", "Kovács"));
        assert_eq!(split("Kovács Anna", true), parts("Anna", "Kovács"));
        assert_eq!(
            split("  Anna   Mária  Kovács ", false),
            parts("Anna Mária", "Kovács")
        );
        assert_eq!(
            split("Kovács Anna Mária", true),
            parts("Anna Mária", "Kovács")
        );
        assert_eq!(split("Anna", false), parts("Anna", ""));
        assert_eq!(
            split("Anna", true),
            parts("Anna", ""),
            "one word is a given name"
        );
        assert_eq!(split("   ", false), parts("", ""));
        assert_eq!(
            split("A\u{301}rpa\u{301}d Nagy", false),
            parts("Árpád", "Nagy"),
            "composed"
        );
    }

    #[test]
    fn display_names_in_each_format() {
        let anna = NameParts::new("Anna", "Kovács");
        assert_eq!(anna.display(Full), "Anna Kovács");
        assert_eq!(anna.display(FirstInitial), "Anna K.");
        let accented = NameParts::new(" Zoltán ", " Ötvös ");
        assert_eq!(accented.display(Full), "Zoltán Ötvös");
        assert_eq!(accented.display(FirstInitial), "Zoltán Ö.");
        assert_eq!(
            NameParts::new("Zoltán", "O\u{308}tvös").display(FirstInitial),
            "Zoltán Ö.",
            "a decomposed initial composes first, so the accent stays"
        );
        assert_eq!(
            NameParts::new("anna", "kovács").display(FirstInitial),
            "anna k.",
            "as written: the list's spelling isn't changed"
        );
        // Either part may be missing.
        assert_eq!(NameParts::new("Anna", "").display(FirstInitial), "Anna");
        assert_eq!(NameParts::new("Anna", "").display(Full), "Anna");
        assert_eq!(NameParts::new("", "Kovács").display(FirstInitial), "Kovács");
        assert_eq!(NameParts::new("", "Kovács").display(Full), "Kovács");
        assert_eq!(NameParts::new("", "").display(Full), "");
        // A family name of a letter is itself, with nothing to abbreviate.
        assert_eq!(NameParts::new("Anna", "K").display(FirstInitial), "Anna K");
        assert_eq!(
            NameParts::new("Anna", "García-López").display(FirstInitial),
            "Anna G."
        );
    }

    #[test]
    fn abbreviations() {
        assert_eq!(abbreviated("Kovács", 1), "K.");
        assert_eq!(abbreviated("Kovács", 2), "Ko.");
        assert_eq!(abbreviated("Kovács", 3), "Kov.");
        assert_eq!(abbreviated("Ötvös", 2), "Öt.");
        assert_eq!(abbreviated("Ko", 2), "Ko", "all of it: no stop");
        assert_eq!(abbreviated("Ko", 3), "Ko");
        assert_eq!(abbreviated("O'Brien", 1), "O.");
        assert_eq!(abbreviated("O'Brien", 2), "O'B.");
        assert_eq!(abbreviated("Li-Wu", 2), "Li.", "no hyphen before the stop");
        assert_eq!(
            abbreviated("Z\u{0323}\u{0307}ab", 1),
            "Z\u{0323}\u{0307}.",
            "a letter keeps its marks"
        );
    }

    #[test]
    fn keys() {
        assert_eq!(name_key("Anna  K."), "anna k.");
        assert_eq!(name_key(" ANNA K. "), name_key("anna k."));
        assert_eq!(name_key("A\u{301}rpa\u{301}d"), name_key("Árpád"));
        assert_ne!(name_key("Anna K."), name_key("Anna K"));
        assert_ne!(name_key("Árpád"), name_key("Arpad"), "accents matter");
    }

    fn parts(names: &[(&str, &str)]) -> Vec<NameParts> {
        names.iter().map(|(g, f)| NameParts::new(g, f)).collect()
    }

    fn unique(taken: &[&str], rows: &[(&str, &str)], format: NameFormat) -> Vec<String> {
        unique_names(taken.iter().copied(), &parts(rows), format)
    }

    #[test]
    fn full_names_get_numbers() {
        assert_eq!(
            unique(&[], &[("Anna", "Kovács"), ("Bence", "Nagy")], Full),
            ["Anna Kovács", "Bence Nagy"],
            "nothing to do"
        );
        assert_eq!(
            unique(
                &[],
                &[
                    ("Anna", "Kovács"),
                    ("Bence", "Nagy"),
                    ("Anna", "Kovács"),
                    ("Anna", "Kovács")
                ],
                Full
            ),
            [
                "Anna Kovács",
                "Bence Nagy",
                "Anna Kovács (2)",
                "Anna Kovács (3)"
            ],
            "triple namesakes: the first keeps the name"
        );
        assert_eq!(
            unique(&["anna  KOVÁCS"], &[("Anna", "Kovács")], Full),
            ["Anna Kovács (2)"],
            "someone in the raffle has it, whatever the case or spacing"
        );
        assert_eq!(
            unique(
                &["Anna Kovács", "Anna Kovács (2)"],
                &[("Anna", "Kovács"), ("Anna", "Kovács")],
                Full
            ),
            ["Anna Kovács (3)", "Anna Kovács (4)"],
            "numbers already taken are skipped"
        );
        assert_eq!(
            unique(&["Anna Kovács (3)"], &[("Anna", "Kovács"); 5], Full),
            [
                "Anna Kovács",
                "Anna Kovács (2)",
                "Anna Kovács (4)",
                "Anna Kovács (5)",
                "Anna Kovács (6)"
            ],
            "many namesakes: each its own number, in order, past a taken one"
        );
        assert_eq!(
            unique(
                &[],
                &[
                    ("Anna", "Kovács"),
                    ("Anna", "Kovács"),
                    ("Anna Kovács", "(2)")
                ],
                Full
            ),
            ["Anna Kovács", "Anna Kovács (3)", "Anna Kovács (2)"],
            "a row's own name is never taken by another's number"
        );
        assert_eq!(
            unique(
                &[],
                &[("Árpád", "Nagy"), ("A\u{301}rpa\u{301}d", "nagy")],
                Full
            ),
            ["Árpád Nagy", "Árpád nagy (2)"],
            "composed and case-insensitive"
        );
        assert_eq!(
            unique(&[], &[("Anna", ""), ("Anna", "")], Full),
            ["Anna", "Anna (2)"],
            "no family name"
        );
    }

    #[test]
    fn initials_take_more_letters_first() {
        assert_eq!(
            unique(&[], &[("Anna", "Kovács"), ("Anna", "Kiss")], FirstInitial),
            ["Anna Ko.", "Anna Ki."],
            "both expand: neither keeps “Anna K.” as if it were hers"
        );
        assert_eq!(
            unique(&[], &[("Anna", "Kovács"), ("Anna", "Kozma")], FirstInitial),
            ["Anna Kov.", "Anna Koz."]
        );
        assert_eq!(
            unique(&[], &[("Anna", "Kovács"), ("Anna", "Kovách")], FirstInitial),
            ["Anna K.", "Anna K. (2)"],
            "three letters at most: if they can't tell two apart, a number on the short name"
        );
        assert_eq!(
            unique(
                &[],
                &[("Anna", "Kovács"), ("Anna", "Kozma"), ("Anna", "Kovách")],
                FirstInitial
            ),
            ["Anna Kov.", "Anna Koz.", "Anna Kov. (2)"]
        );
        assert_eq!(
            unique(&[], &[("Anna", "Kovács"), ("Anna", "Kovács")], FirstInitial),
            ["Anna K.", "Anna K. (2)"],
            "letters can't tell namesakes apart: a number, on the short name"
        );
        assert_eq!(
            unique(
                &[],
                &[("Anna", "Kovács"), ("Anna", "Kovács"), ("Anna", "Kiss")],
                FirstInitial
            ),
            ["Anna Ko.", "Anna Ko. (2)", "Anna Ki."]
        );
        assert_eq!(
            unique(&["Anna K."], &[("Anna", "Kovács")], FirstInitial),
            ["Anna Ko."],
            "someone who joined as “Anna K.” keeps it; the new row moves"
        );
        assert_eq!(
            unique(
                &["Anna K.", "anna ko."],
                &[("Anna", "Kovács")],
                FirstInitial
            ),
            ["Anna Kov."]
        );
        assert_eq!(
            unique(
                &["Anna K.", "Anna Ko.", "Anna Kov."],
                &[("Anna", "Kovács")],
                FirstInitial
            ),
            ["Anna Kov. (2)"]
        );
        assert_eq!(
            unique(&["Anna K"], &[("Anna", "K")], FirstInitial),
            ["Anna K (2)"],
            "nothing more to show"
        );
        assert_eq!(
            unique(
                &[],
                &[("Anna", ""), ("Anna", "Kovács"), ("Anna", "")],
                FirstInitial
            ),
            ["Anna", "Anna K.", "Anna (2)"],
            "no family name: a number"
        );
        assert_eq!(
            unique(&[], &[("Ödön", "Ötvös"), ("Ödön", "Őri")], FirstInitial),
            ["Ödön Ö.", "Ödön Ő."],
            "Hungarian accents are letters of their own"
        );
        assert_eq!(
            unique(&[], &[("Anna", "Kovács"), ("Anna", "Ko")], FirstInitial),
            ["Anna Ko.", "Anna Ko"],
            "a family name shown whole has no stop"
        );
        assert_eq!(
            unique(&[], &[("Bence", "Nagy"), ("Anna", "Kovács")], FirstInitial),
            ["Bence N.", "Anna K."],
            "only what collides changes"
        );
    }

    #[test]
    fn numbers_fit_the_limit() {
        let long = "x".repeat(MAX_ADDED_NAME_CHARS - 1);
        let names = unique(&[], &[(long.as_str(), ""), (long.as_str(), "")], Full);
        assert_eq!(names[0], long);
        assert_eq!(
            names[1],
            format!("{} (2)", "x".repeat(MAX_ADDED_NAME_CHARS - 4))
        );
        assert_eq!(names[1].chars().count(), MAX_ADDED_NAME_CHARS);
        // Cut by whole characters, accents included, and no space left
        // before the number.
        let accented = format!("{} {}", "é".repeat(59), "ő".repeat(4));
        assert_eq!(accented.chars().count(), MAX_ADDED_NAME_CHARS);
        let names = unique(&[accented.as_str()], &[(accented.as_str(), "")], Full);
        assert_eq!(names[0], format!("{} (2)", "é".repeat(59)));
        assert!(crate::entrant::added_name(&names[0]).is_ok());
        // Ten namesakes: "(10)" is a character longer.
        let rows: Vec<(&str, &str)> = vec![(long.as_str(), ""); 10];
        let names = unique(&[], &rows, Full);
        assert_eq!(
            names
                .iter()
                .map(|n| name_key(n))
                .collect::<HashSet<_>>()
                .len(),
            10
        );
        assert!(
            names
                .iter()
                .all(|n| n.chars().count() <= MAX_ADDED_NAME_CHARS)
        );
        assert!(names[9].ends_with(" (10)"), "{}", names[9]);
        // A name too long already is left for the length check.
        let too_long = "x".repeat(MAX_ADDED_NAME_CHARS + 1);
        let names = unique(&[too_long.as_str()], &[(too_long.as_str(), "")], Full);
        assert_eq!(names[0], format!("{too_long} (2)"));
    }

    #[test]
    fn unique_names_are_unique() {
        let taken = ["Anna K.", "Bence", "Anna Ko. (2)"];
        let rows = [
            ("Anna", "Kovács"),
            ("Anna", "Kovács"),
            ("Anna", "Kiss"),
            ("Anna", "Kozma"),
            ("Bence", ""),
            ("Bence", ""),
            ("anna", "kovács"),
        ];
        for format in NameFormat::ALL {
            let names = unique(&taken, &rows, format);
            let mut keys: HashSet<String> = taken.iter().map(|t| name_key(t)).collect();
            for name in &names {
                assert!(
                    keys.insert(name_key(name)),
                    "{format:?}: {name} twice in {names:?}"
                );
            }
            assert!(
                name_clashes(
                    taken,
                    names.iter().enumerate().map(|(i, n)| (i, n.as_str()))
                )
                .is_empty(),
                "{format:?}: what the browser makes, the server takes"
            );
        }
    }

    #[test]
    fn clashes() {
        let clash = |row, error, shown_as: &str| NameClash {
            row,
            error,
            shown_as: shown_as.to_owned(),
        };
        assert_eq!(
            name_clashes(
                ["Anna K.", "Brave Otter"],
                [
                    (0, "Bence"),
                    (2, "anna  k."),
                    (3, "Bence"),
                    (5, "BENCE"),
                    (6, "Csilla")
                ]
            ),
            [
                clash(2, RowError::NameTaken, "Anna K."),
                clash(3, RowError::DuplicateName, "Bence"),
                clash(5, RowError::DuplicateName, "Bence"),
            ],
            "by the key; the later of two rows; the name as the other has it"
        );
        assert_eq!(
            name_clashes(["Árpád"], [(0, "A\u{301}rpa\u{301}d")]),
            [clash(0, RowError::NameTaken, "Árpád")],
            "composed first"
        );
        assert!(name_clashes(["Anna K."], [(0, "Anna Ko.")]).is_empty());
    }
}
