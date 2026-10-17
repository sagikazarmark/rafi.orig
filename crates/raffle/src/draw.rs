//! One draw: selecting the winner of one item. It is announced (committed
//! to a future drand round) and pending until its round's beacon completes it;
//! then the organizer resolves its winner.

use serde::{Deserialize, Serialize};

use crate::{
    command::Resolution,
    id::{EntrantId, ItemId},
};

/// A draw that has been announced but not completed yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDraw {
    pub k: u64,
    pub item: ItemId,
    pub round: u64,
    pub round_time: u64,
}

/// A draw that picked its winner, and where that winner stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedDraw {
    pub k: u64,
    pub item: ItemId,
    pub round: u64,
    pub pass: u8,
    /// How many entrants the pool held.
    pub pool_size: usize,
    /// The winner's position in the pool.
    pub index: usize,
    pub entrant: EntrantId,
    pub outcome: Outcome,
}

impl CompletedDraw {
    /// Whether the winner forfeited ([`Outcome::is_forfeit`]): out of pass 2.
    pub fn forfeited(&self) -> bool {
        self.outcome.is_forfeit()
    }
}

/// Where a draw's winner stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Drawn, not yet resolved by the organizer.
    Drawn,
    Claimed,
    NotHere,
    Declined,
}

impl Outcome {
    /// The outcome's name, as serialized (`"not_here"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Drawn => "drawn",
            Self::Claimed => "claimed",
            Self::NotHere => "not_here",
            Self::Declined => "declined",
        }
    }

    /// Whether the winner gave the item up (not here, or declined): it is
    /// drawn again, and they are out of pass 2. The one definition of a
    /// forfeit.
    pub fn is_forfeit(self) -> bool {
        matches!(self, Self::NotHere | Self::Declined)
    }
}

impl From<Resolution> for Outcome {
    fn from(resolution: Resolution) -> Self {
        match resolution {
            Resolution::Claimed => Self::Claimed,
            Resolution::NotHere => Self::NotHere,
            Resolution::Declined => Self::Declined,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_the_serialized_ones() {
        for outcome in [
            Outcome::Drawn,
            Outcome::Claimed,
            Outcome::NotHere,
            Outcome::Declined,
        ] {
            assert_eq!(
                serde_json::to_value(outcome).unwrap(),
                outcome.as_str(),
                "{outcome:?}"
            );
        }
    }
}
