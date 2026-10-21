//! What a raffle's settings and prizes may hold, and which of them are fixed
//! once an entrant has accepted them.

use crate::{
    dates::{self, DAY_SECS, Date},
    entrant::Source,
};

/// The longest raffle title or prize name, in characters.
pub const MAX_TITLE_CHARS: usize = 120;
/// The longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 2000;
/// The longest rules text, in characters.
pub const MAX_RULES_CHARS: usize = 5000;
/// The longest privacy text an organizer adds to the notice, in characters.
pub const MAX_PRIVACY_TEXT_CHARS: usize = MAX_RULES_CHARS;
/// The longest URL, in characters.
pub const MAX_URL_CHARS: usize = 2000;
/// The most copies of one prize: each is an item of the raffle.
pub const MAX_PRIZE_QTY: u32 = 500;

/// The installation's retention settings, as the rules need them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Days after the event date before non-winners are erased, for owners
    /// who set none.
    pub days: i64,
    /// The longest period an owner or a raffle may choose.
    pub max_days: i64,
    /// Days after the erasure date a winner's data is kept, unless their
    /// prize was marked delivered: the hard ceiling.
    pub winner_hold_days: i64,
}

impl RetentionPolicy {
    /// The longest any period may be configured to, in days.
    pub const MAX_MAX_DAYS: i64 = 365;

    /// Whether the settings make sense: nothing negative, the default within
    /// the maximum, the maximum within reason.
    pub fn validate(self) -> Result<(), String> {
        if self.days < 0 || self.max_days < 0 || self.winner_hold_days < 0 {
            return Err("retention periods can't be negative".into());
        }
        if self.max_days > Self::MAX_MAX_DAYS {
            return Err(format!(
                "the longest retention period is {} days",
                Self::MAX_MAX_DAYS
            ));
        }
        if self.days > self.max_days {
            return Err(format!(
                "the default retention period ({} days) is over the maximum ({} days)",
                self.days, self.max_days
            ));
        }
        if self.winner_hold_days > Self::MAX_MAX_DAYS {
            return Err(format!(
                "the longest winner hold is {} days",
                Self::MAX_MAX_DAYS
            ));
        }
        Ok(())
    }

    /// The winner hold in seconds.
    pub fn winner_hold_secs(self) -> i64 {
        self.winner_hold_days * DAY_SECS
    }

    /// When a raffle's held winners are erased at the latest: its erasure
    /// date plus the hold.
    pub fn winners_erase_at(self, erase_at: i64) -> i64 {
        erase_at + self.winner_hold_secs()
    }
}

/// A raffle's settings, as the organizer gave them or as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub organizer_name: &'a str,
    pub contact_email: &'a str,
    /// Where privacy requests go; blank means `contact_email`.
    pub privacy_email: &'a str,
    /// The organizer's own privacy text, shown after the generated notice.
    pub privacy_text: &'a str,
    /// The organizer's own privacy notice, linked after the generated one.
    pub privacy_url: &'a str,
    pub rules: &'a str,
    /// The event's calendar day, `YYYY-MM-DD`.
    pub event_date: &'a str,
    /// The instant the event day ends where the organizer is (their browser
    /// says); `None` for the end of the day in UTC. [`normalize`](Self::normalize)
    /// resolves it.
    pub event_ends_at: Option<i64>,
    /// Days after the event date before non-winners are erased.
    pub retention_days: i64,
}

/// Why settings are refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsError {
    /// Not a [title].
    Title,
    /// The description, the rules or the privacy text are too long ([`texts_fit`]).
    TooLong,
    /// Not a calendar day (`YYYY-MM-DD`).
    EventDate,
    /// The retention period is negative or over the installation's maximum.
    Retention,
    /// Not an absolute `https` URL.
    PrivacyUrl,
}

impl<'a> Settings<'a> {
    /// The settings as they are stored: texts trimmed, the title, the date,
    /// the period (at most `max_retention_days`) and the URL checked, the end
    /// of the event day resolved.
    pub fn normalize(self, max_retention_days: i64) -> Result<Self, SettingsError> {
        if !texts_fit(self.description, self.rules) || !privacy_text_fits(self.privacy_text) {
            return Err(SettingsError::TooLong);
        }
        let date = Date::parse(self.event_date).ok_or(SettingsError::EventDate)?;
        if !retention_days_ok(self.retention_days, max_retention_days) {
            return Err(SettingsError::Retention);
        }
        let privacy_url = url(self.privacy_url).ok_or(SettingsError::PrivacyUrl)?;
        Ok(Self {
            title: title(self.title).ok_or(SettingsError::Title)?,
            description: self.description.trim(),
            organizer_name: self.organizer_name.trim(),
            contact_email: self.contact_email.trim(),
            privacy_email: self.privacy_email.trim(),
            privacy_text: self.privacy_text.trim(),
            privacy_url,
            rules: self.rules.trim(),
            event_date: self.event_date.trim(),
            event_ends_at: Some(dates::ends_at(date, self.event_ends_at)),
            retention_days: self.retention_days,
        })
    }

    /// The event's day. Panics on settings that weren't normalized.
    pub fn event(&self) -> Date {
        Date::parse(self.event_date).expect("normalized settings have a date")
    }

    /// When non-winners are erased: the end of the event day plus the
    /// retention period. Only meaningful once normalized.
    pub fn erase_at(&self) -> i64 {
        erase_at(
            self.event_ends_at
                .unwrap_or_else(|| self.event().ends_utc()),
            self.retention_days,
        )
    }

    /// The fields that lock once an entrant has accepted the rules.
    pub fn fixed(&self) -> Fixed<'a> {
        Fixed {
            rules: self.rules,
            event_date: self.event_date,
            retention_days: self.retention_days,
            privacy_text: self.privacy_text,
            privacy_url: self.privacy_url,
        }
    }
}

/// When non-winners are erased: `event_ends_at` plus `retention_days`.
pub fn erase_at(event_ends_at: i64, retention_days: i64) -> i64 {
    event_ends_at + retention_days * DAY_SECS
}

/// The erasure date as a calendar day, for entrants: the event date plus the
/// period.
pub fn erase_date(event: Date, retention_days: i64) -> Date {
    event.plus_days(retention_days)
}

/// A raffle's erasure dates as calendar days, as entrants and organizers
/// read them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErasureDates {
    /// The event day.
    pub event: Date,
    /// The day non-winners are erased.
    pub erase: Date,
    /// The day winners whose prize wasn't delivered are erased at the latest.
    pub winners_erase: Date,
}

impl ErasureDates {
    /// The dates of a raffle on `event_date` with `retention_days`, under a
    /// policy with `winner_hold_days`; `None` if the date doesn't parse
    /// (a stored one always does).
    pub fn of(event_date: &str, retention_days: i64, winner_hold_days: i64) -> Option<Self> {
        let event = Date::parse(event_date)?;
        Some(Self {
            event,
            erase: erase_date(event, retention_days),
            winners_erase: erase_date(event, retention_days + winner_hold_days),
        })
    }
}

/// The settings that are fixed once a real entrant has accepted the rules:
/// what the join form and the notices showed them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixed<'a> {
    pub rules: &'a str,
    pub event_date: &'a str,
    pub retention_days: i64,
    pub privacy_text: &'a str,
    pub privacy_url: &'a str,
}

impl Fixed<'_> {
    /// The fields that differ between `self` and `other`, by the names the
    /// organizer knows them by.
    pub fn changed_from(&self, other: &Self) -> Vec<&'static str> {
        let mut changed = Vec::new();
        if self.rules != other.rules {
            changed.push("additional rules");
        }
        if self.event_date != other.event_date {
            changed.push("event date");
        }
        if self.retention_days != other.retention_days {
            changed.push("retention period");
        }
        if self.privacy_text != other.privacy_text {
            changed.push("privacy text");
        }
        if self.privacy_url != other.privacy_url {
            changed.push("privacy notice link");
        }
        changed
    }
}

/// What an owner's new raffles start with: the settings that are usually the
/// same for every raffle they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults<'a> {
    pub organizer_name: &'a str,
    pub contact_email: &'a str,
    pub privacy_email: &'a str,
    pub privacy_text: &'a str,
    pub privacy_url: &'a str,
    pub rules: &'a str,
    /// `None`: the installation's default.
    pub retention_days: Option<i64>,
}

impl<'a> Defaults<'a> {
    /// The defaults as they are stored: texts trimmed, the rules, the period
    /// and the URL checked as a raffle's are.
    pub fn normalize(self, max_retention_days: i64) -> Result<Self, SettingsError> {
        if !texts_fit("", self.rules) || !privacy_text_fits(self.privacy_text) {
            return Err(SettingsError::TooLong);
        }
        if let Some(days) = self.retention_days
            && !retention_days_ok(days, max_retention_days)
        {
            return Err(SettingsError::Retention);
        }
        Ok(Self {
            organizer_name: self.organizer_name.trim(),
            contact_email: self.contact_email.trim(),
            privacy_email: self.privacy_email.trim(),
            privacy_text: self.privacy_text.trim(),
            privacy_url: url(self.privacy_url).ok_or(SettingsError::PrivacyUrl)?,
            rules: self.rules.trim(),
            retention_days: self.retention_days,
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

fn privacy_text_fits(text: &str) -> bool {
    text.chars().count() <= MAX_PRIVACY_TEXT_CHARS
}

/// Whether a retention period can be chosen: within the installation's
/// maximum.
pub fn retention_days_ok(days: i64, max: i64) -> bool {
    (0..=max).contains(&days)
}

/// `input` trimmed, if it is blank or an absolute `https` URL.
pub fn url(input: &str) -> Option<&str> {
    let url = input.trim();
    if url.is_empty() {
        return Some(url);
    }
    let ok = url.len() > "https://".len()
        && url.starts_with("https://")
        && url.chars().count() <= MAX_URL_CHARS
        && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then_some(url)
}

/// Whether a prize can have `qty` copies.
pub fn prize_qty(qty: u32) -> bool {
    (1..=MAX_PRIZE_QTY).contains(&qty)
}

/// Whether a raffle's fixed settings ([`Fixed`]) can still change, given
/// its entrants' sources: not once anyone has accepted them
/// ([`Source::accepted_rules`]). Fakes don't count, so a rehearsal leaves
/// them editable.
pub fn rules_editable(entrants: impl IntoIterator<Item = Source>) -> bool {
    !entrants.into_iter().any(Source::accepted_rules)
}

/// What a raffle still needs before registration can open: the privacy
/// notice must name a controller and a way to reach them. Empty when it may
/// open.
pub fn missing_for_open(
    organizer_name: &str,
    contact_email: &str,
    privacy_email: &str,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if organizer_name.trim().is_empty() {
        missing.push("the organizer's name");
    }
    if contact_email.trim().is_empty() && privacy_email.trim().is_empty() {
        missing.push("a contact or privacy email");
    }
    missing
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

    const MAX: i64 = 30;

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
            privacy_email: " dpo@x.org ",
            privacy_text: " Our policy. ",
            privacy_url: "",
            rules: "",
            event_date: "2026-10-28",
            event_ends_at: None,
            retention_days: 7,
        }
    }

    #[test]
    fn settings_normalize() {
        let s = settings(" Meetup ").normalize(MAX).unwrap();
        assert_eq!(s.title, "Meetup");
        assert_eq!(s.description, "About");
        assert_eq!(s.organizer_name, "Org");
        assert_eq!(s.privacy_email, "dpo@x.org");
        assert_eq!(s.privacy_text, "Our policy.");
        assert_eq!(
            s.event_ends_at,
            Some(Date::parse("2026-10-28").unwrap().ends_utc()),
            "the end of the day in UTC when the browser said nothing"
        );
        assert_eq!(s.normalize(MAX), Ok(s), "normalizing is idempotent");
        assert_eq!(settings(" ").normalize(MAX), Err(SettingsError::Title));
        let long = "x".repeat(MAX_RULES_CHARS + 1);
        let s = Settings {
            rules: &long,
            ..settings("Meetup")
        };
        assert_eq!(s.normalize(MAX), Err(SettingsError::TooLong));
        let s = Settings {
            privacy_text: &long,
            ..settings("Meetup")
        };
        assert_eq!(s.normalize(MAX), Err(SettingsError::TooLong));
    }

    #[test]
    fn the_event_date_the_period_and_the_url_are_checked() {
        let bad_date = Settings {
            event_date: "next Tuesday",
            ..settings("Meetup")
        };
        assert_eq!(bad_date.normalize(MAX), Err(SettingsError::EventDate));
        let too_long = Settings {
            retention_days: MAX + 1,
            ..settings("Meetup")
        };
        assert_eq!(too_long.normalize(MAX), Err(SettingsError::Retention));
        let negative = Settings {
            retention_days: -1,
            ..settings("Meetup")
        };
        assert_eq!(negative.normalize(MAX), Err(SettingsError::Retention));
        let zero = Settings {
            retention_days: 0,
            ..settings("Meetup")
        };
        assert!(zero.normalize(MAX).is_ok(), "erased on the event day");
        for bad in ["http://x.org", "x.org", "https://", "https://x.org/a b"] {
            let s = Settings {
                privacy_url: bad,
                ..settings("Meetup")
            };
            assert_eq!(s.normalize(MAX), Err(SettingsError::PrivacyUrl), "{bad}");
        }
        let ok = Settings {
            privacy_url: " https://x.org/privacy ",
            ..settings("Meetup")
        };
        assert_eq!(
            ok.normalize(MAX).unwrap().privacy_url,
            "https://x.org/privacy"
        );
    }

    #[test]
    fn the_erasure_date_follows_the_event() {
        let s = settings("Meetup").normalize(MAX).unwrap();
        let event = Date::parse("2026-10-28").unwrap();
        assert_eq!(s.erase_at(), event.ends_utc() + 7 * DAY_SECS);
        assert_eq!(erase_date(event, 7).iso(), "2026-11-04");
        let dates = ErasureDates::of("2026-10-28", 7, 90).unwrap();
        assert_eq!(
            (dates.erase.iso(), dates.winners_erase.iso()),
            ("2026-11-04".to_owned(), "2027-02-02".to_owned())
        );
        assert!(ErasureDates::of("soon", 7, 90).is_none());
        // The organizer's browser says the day ends earlier (east of UTC).
        let cet = Settings {
            event_ends_at: Some(event.ends_utc() - 3600),
            ..settings("Meetup")
        }
        .normalize(MAX)
        .unwrap();
        assert_eq!(cet.erase_at(), event.ends_utc() - 3600 + 7 * DAY_SECS);
    }

    #[test]
    fn fixed_fields_are_named() {
        let a = settings("Meetup").normalize(MAX).unwrap();
        let b = Settings {
            rules: "Be nice.",
            retention_days: 3,
            ..a
        };
        assert_eq!(
            b.fixed().changed_from(&a.fixed()),
            ["additional rules", "retention period"]
        );
        let c = Settings {
            title: "Other",
            description: "Other",
            ..a
        };
        assert!(c.fixed().changed_from(&a.fixed()).is_empty());
    }

    #[test]
    fn opening_needs_a_controller_and_a_contact() {
        assert!(missing_for_open("Org", "a@b.c", "").is_empty());
        assert!(missing_for_open("Org", "", "dpo@b.c").is_empty());
        assert_eq!(
            missing_for_open(" ", "", ""),
            ["the organizer's name", "a contact or privacy email"]
        );
        assert_eq!(
            missing_for_open("Org", " ", ""),
            ["a contact or privacy email"]
        );
    }

    #[test]
    fn retention_settings_validate() {
        let ok = RetentionPolicy {
            days: 7,
            max_days: 30,
            winner_hold_days: 90,
        };
        assert!(ok.validate().is_ok());
        assert!(RetentionPolicy { days: 31, ..ok }.validate().is_err());
        assert!(RetentionPolicy { days: -1, ..ok }.validate().is_err());
        assert!(
            RetentionPolicy {
                max_days: 366,
                ..ok
            }
            .validate()
            .is_err()
        );
        assert!(
            RetentionPolicy {
                winner_hold_days: 400,
                ..ok
            }
            .validate()
            .is_err()
        );
        assert_eq!(ok.winner_hold_secs(), 90 * DAY_SECS);
        assert_eq!(ok.winners_erase_at(1000), 1000 + 90 * DAY_SECS);
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
            privacy_email: "",
            privacy_text: "",
            privacy_url: " https://x.org/p ",
            rules: " Rules. ",
            retention_days: Some(3),
        };
        assert_eq!(
            d.normalize(MAX),
            Ok(Defaults {
                organizer_name: "Org",
                contact_email: "a@b.c",
                privacy_email: "",
                privacy_text: "",
                privacy_url: "https://x.org/p",
                rules: "Rules.",
                retention_days: Some(3),
            })
        );
        let long = "x".repeat(MAX_RULES_CHARS + 1);
        let too_long = Defaults { rules: &long, ..d };
        assert_eq!(too_long.normalize(MAX), Err(SettingsError::TooLong));
        let over = Defaults {
            retention_days: Some(MAX + 1),
            ..d
        };
        assert_eq!(over.normalize(MAX), Err(SettingsError::Retention));
        let unset = Defaults {
            retention_days: None,
            ..d
        };
        assert!(unset.normalize(MAX).is_ok());
    }
}
