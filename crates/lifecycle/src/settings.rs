//! What a raffle's settings and prizes may hold.

use crate::entrant::Source;

/// The longest raffle title or prize name, in characters.
pub const MAX_TITLE_CHARS: usize = 120;
/// The longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 2000;
/// The longest rules text, in characters.
pub const MAX_RULES_CHARS: usize = 5000;
/// The most copies of one prize: each is an item of the raffle.
pub const MAX_PRIZE_QTY: u32 = 500;

/// A raffle's settings, as the organizer gave them or as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub organizer_name: &'a str,
    pub contact_email: &'a str,
    pub rules: &'a str,
    pub event_date: &'a str,
}

/// Why settings are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsError {
    /// Not a [title].
    Title,
    /// The description or the rules are too long ([`texts_fit`]).
    TooLong,
}

impl<'a> Settings<'a> {
    /// The settings as they are stored: texts trimmed and the title checked.
    pub fn normalize(self) -> Result<Self, SettingsError> {
        if !texts_fit(self.description, self.rules) {
            return Err(SettingsError::TooLong);
        }
        Ok(Self {
            title: title(self.title).ok_or(SettingsError::Title)?,
            description: self.description.trim(),
            organizer_name: self.organizer_name.trim(),
            contact_email: self.contact_email.trim(),
            rules: self.rules.trim(),
            event_date: self.event_date.trim(),
        })
    }
}

/// What an owner's new raffles start with: the settings that are usually the
/// same for every raffle they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults<'a> {
    pub organizer_name: &'a str,
    pub contact_email: &'a str,
    pub rules: &'a str,
}

impl<'a> Defaults<'a> {
    /// The defaults as they are stored: texts trimmed, the rules checked as a
    /// raffle's are.
    pub fn normalize(self) -> Result<Self, SettingsError> {
        if !texts_fit("", self.rules) {
            return Err(SettingsError::TooLong);
        }
        Ok(Self {
            organizer_name: self.organizer_name.trim(),
            contact_email: self.contact_email.trim(),
            rules: self.rules.trim(),
        })
    }
}

/// `input` trimmed, if it is a valid title (or prize name): 1 to
/// [`MAX_TITLE_CHARS`] characters.
pub fn title(input: &str) -> Option<&str> {
    let title = input.trim();
    (!title.is_empty() && title.chars().count() <= MAX_TITLE_CHARS).then_some(title)
}

/// Whether a description and rules fit.
pub fn texts_fit(description: &str, rules: &str) -> bool {
    description.chars().count() <= MAX_DESCRIPTION_CHARS && rules.chars().count() <= MAX_RULES_CHARS
}

/// Whether a prize can have `qty` copies.
pub fn prize_qty(qty: u32) -> bool {
    (1..=MAX_PRIZE_QTY).contains(&qty)
}

/// Whether a raffle's additional rules can still change, given its entrants'
/// sources: not once anyone has accepted them ([`Source::accepted_rules`]).
/// Fakes don't count, so a rehearsal leaves them editable.
pub fn rules_editable(entrants: impl IntoIterator<Item = Source>) -> bool {
    !entrants.into_iter().any(Source::accepted_rules)
}

/// The title of a raffle copied from one titled `title`: still a valid title.
pub fn copy_title(title: &str) -> String {
    const SUFFIX: &str = " (copy)";
    let keep = MAX_TITLE_CHARS - SUFFIX.chars().count();
    format!(
        "{}{SUFFIX}",
        title.trim().chars().take(keep).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title("  Meetup "), Some("Meetup"));
        assert_eq!(title("   "), None);
        assert_eq!(title(&"x".repeat(MAX_TITLE_CHARS + 1)), None);
    }

    #[test]
    fn a_copy_title_is_a_title() {
        let long = "x".repeat(MAX_TITLE_CHARS);
        assert!(title(&copy_title(&long)).is_some());
        assert_eq!(copy_title("Meetup"), "Meetup (copy)");
    }

    fn settings(title: &str) -> Settings<'_> {
        Settings {
            title,
            description: " About ",
            organizer_name: " Org ",
            contact_email: "",
            rules: "",
            event_date: "",
        }
    }

    #[test]
    fn settings_normalize() {
        let s = settings(" Meetup ").normalize().unwrap();
        assert_eq!(s.title, "Meetup");
        assert_eq!(s.description, "About");
        assert_eq!(s.organizer_name, "Org");
        assert_eq!(s.normalize(), Ok(s), "normalizing is idempotent");
        assert_eq!(settings(" ").normalize(), Err(SettingsError::Title));
        let long = "x".repeat(MAX_RULES_CHARS + 1);
        let s = Settings {
            rules: &long,
            ..settings("Meetup")
        };
        assert_eq!(s.normalize(), Err(SettingsError::TooLong));
    }

    #[test]
    fn texts_fit_their_limits() {
        let at = |n| "é".repeat(n);
        assert!(texts_fit(&at(MAX_DESCRIPTION_CHARS), &at(MAX_RULES_CHARS)));
        assert!(!texts_fit(&at(MAX_DESCRIPTION_CHARS + 1), ""));
        assert!(!texts_fit("", &at(MAX_RULES_CHARS + 1)));
    }

    #[test]
    fn quantities() {
        assert!(!prize_qty(0));
        assert!(prize_qty(1));
        assert!(prize_qty(MAX_PRIZE_QTY));
        assert!(!prize_qty(MAX_PRIZE_QTY + 1));
    }

    #[test]
    fn rules_are_fixed_once_accepted() {
        assert!(rules_editable([]));
        assert!(rules_editable([Source::Fake, Source::Fake]));
        assert!(!rules_editable([Source::Fake, Source::SelfService]));
        assert!(!rules_editable([Source::Manual]));
    }

    #[test]
    fn defaults_normalize() {
        let d = Defaults {
            organizer_name: " Org ",
            contact_email: " a@b.c ",
            rules: " Rules. ",
        };
        assert_eq!(
            d.normalize(),
            Ok(Defaults {
                organizer_name: "Org",
                contact_email: "a@b.c",
                rules: "Rules.",
            })
        );
        let long = "x".repeat(MAX_RULES_CHARS + 1);
        let d = Defaults { rules: &long, ..d };
        assert_eq!(d.normalize(), Err(SettingsError::TooLong));
    }
}
