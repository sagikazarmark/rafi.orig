use serde::{Deserialize, Serialize};

/// How an entrant got in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Joined on the participant page.
    #[serde(rename = "self")]
    SelfService,
    /// Added by the organizer.
    Manual,
    /// Made up to try a raffle out (test mode only: [`fakes`](super::fakes)).
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

    /// Whether removing an entrant from here at their request (before the
    /// freeze) is remembered, so that a later add of the organizer's leaves
    /// them out unless the organizer lifts it for that row: anyone real, a
    /// joiner who asks too, since the list the organizer adds next may have
    /// them. A fake is nobody to remember. Exhaustive, so a new source has
    /// to decide.
    pub fn removal_remembered(self) -> bool {
        match self {
            Self::SelfService | Self::Manual => true,
            Self::Fake => false,
        }
    }

    /// Whether an entrant from here can take themselves off after the
    /// freeze ([`may_object`](super::may_object)): someone the organizer
    /// added, who accepted nothing and is in on the organizer's legitimate
    /// interest, which they may object to (GDPR Art. 21). Someone who joined
    /// accepted the rules, and sharing their entry with the event until the
    /// finish is one of them; a fake is nobody. Exhaustive, so a new source
    /// has to decide.
    pub fn may_object(self) -> bool {
        match self {
            Self::Manual => true,
            Self::SelfService | Self::Fake => false,
        }
    }

    /// Whether a winner from here has no phone in the raffle: nothing tells
    /// them they won, and they have no claim code to show. Someone the
    /// organizer added: the projector calls them to the front, and the draw
    /// console has the organizer check it is them (the email they
    /// registered with, their badge) and confirm it, instead of comparing a
    /// code. A joiner's phone says "YOU WON!" and shows the code; a fake
    /// stands in for a joiner in a rehearsal. Exhaustive, so a new source
    /// has to decide.
    pub fn no_phone(self) -> bool {
        match self {
            Self::Manual => true,
            Self::SelfService | Self::Fake => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only someone the organizer added has no phone to tell them they
    /// won: a joiner's says so, and a fake stands in for a joiner.
    #[test]
    fn who_has_no_phone() {
        let no_phone: Vec<Source> = Source::ALL.into_iter().filter(|s| s.no_phone()).collect();
        assert_eq!(no_phone, [Source::Manual]);
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
    fn removals_at_their_request_are_remembered_for_real_people() {
        assert!(Source::SelfService.removal_remembered());
        assert!(Source::Manual.removal_remembered());
        assert!(!Source::Fake.removal_remembered());
    }
}
