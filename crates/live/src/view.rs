//! The participant view: everything a phone needs, and nothing more.

use serde::{Deserialize, Serialize};

/// A raffle's lifecycle state, as phones see it.
///
/// Mirrors `rafi_core::raffle::RaffleState` variant for variant, with the same
/// serde names, so the server maps one to the other one to one. It lives here
/// so the phone doesn't depend on `rafi-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Being prepared; not visible to the public.
    Draft,
    /// Published: the intro screen is visible, registration not yet open.
    Ready,
    /// Registration is open.
    Open,
    /// Registration is closed (can be reopened).
    Closed,
    /// The entrant list is frozen and committed; drawing can start.
    Frozen,
    /// At least one draw happened.
    Drawing,
    Finished,
    /// Personal data has been removed.
    Purged,
}

impl Phase {
    /// Whether the raffle has ended: nothing a phone shows changes after this.
    pub fn is_over(self) -> bool {
        matches!(self, Self::Finished | Self::Purged)
    }

    /// Whether the raffle is public and not over, which is when a page
    /// subscribes to [`RaffleLive`](crate::RaffleLive). Ready is included so a
    /// visitor's page can reload when registration opens.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Open | Self::Closed | Self::Frozen | Self::Drawing
        )
    }
}

/// What a phone needs, and nothing more.
///
/// The Durable Object builds it from one consistent read of the database (see
/// [`ParticipantView::new`]) and publishes it on [`RaffleLive`](crate::RaffleLive);
/// the server also embeds it in the attendee page. It has no time-dependent
/// fields, so [`v`](Self::v) alone orders views.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParticipantView {
    /// The raffle's version in the database. Orders views; not part of the
    /// content (see [`same_content`](Self::same_content)). Phones accept only
    /// a newer `v`.
    pub v: u64,
    pub state: Phase,
    /// The draw in progress, if any.
    pub current: Option<Current>,
    /// The latest draw that holds its prize: no-shows and declines are
    /// skipped. Its name is the only display name a phone receives.
    pub last: Option<LastDraw>,
    /// Entrants holding a prize (drawn or claimed), so a phone can tell it won.
    pub winners: Vec<Winner>,
    /// Prizes still to be won.
    pub prizes_left: u32,
}

/// The draw in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Current {
    pub item_name: String,
    /// The committed drand round.
    pub round: u64,
    /// Unix seconds the drand round is published.
    pub round_time: u64,
}

/// The latest draw, as shown between draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastDraw {
    pub display_name: String,
    pub item_name: String,
}

/// An entrant holding a prize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Winner {
    pub entrant_id: String,
    pub item_name: String,
    /// The organizer has handed the prize over.
    pub claimed: bool,
}

/// How a drawn entrant's turn ended. Mirrors `rafi_drawing::state::Outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawOutcome {
    /// Drawn, not yet resolved by the organizer.
    Drawn,
    Claimed,
    NotHere,
    Declined,
}

impl DrawOutcome {
    /// Whether the entrant holds the prize (drawn or claimed).
    pub fn holds_prize(self) -> bool {
        matches!(self, Self::Drawn | Self::Claimed)
    }
}

/// One public draw, as input to [`ParticipantView::new`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawInput {
    pub entrant_id: String,
    /// The name shown on the screen (already masked if the organizer hid it).
    pub display_name: String,
    pub item_name: String,
    pub outcome: DrawOutcome,
}

impl ParticipantView {
    /// Builds the view. `draws` are the raffle's draws in chronological order.
    ///
    /// - `winners` are the draws that hold their prize, with `claimed` set for
    ///   claimed ones;
    /// - `last` is the latest of those, so a no-show or a decline is never
    ///   celebrated on phones.
    pub fn new(
        v: u64,
        state: Phase,
        current: Option<Current>,
        draws: impl IntoIterator<Item = DrawInput>,
        prizes_left: u32,
    ) -> Self {
        let mut last = None;
        let mut winners = Vec::new();
        for d in draws {
            if !d.outcome.holds_prize() {
                continue;
            }
            winners.push(Winner {
                entrant_id: d.entrant_id,
                item_name: d.item_name.clone(),
                claimed: d.outcome == DrawOutcome::Claimed,
            });
            last = Some(LastDraw {
                display_name: d.display_name,
                item_name: d.item_name,
            });
        }
        Self {
            v,
            state,
            current,
            last,
            winners,
            prizes_left,
        }
    }

    /// Whether both views show the same thing, ignoring [`v`](Self::v). The
    /// DO publishes a newer version only if its content changed.
    pub fn same_content(&self, other: &Self) -> bool {
        let Self {
            v: _,
            state,
            current,
            last,
            winners,
            prizes_left,
        } = self;
        *state == other.state
            && *current == other.current
            && *last == other.last
            && *winners == other.winners
            && *prizes_left == other.prizes_left
    }

    /// The latest prize `entrant_id` holds, if any.
    pub fn win_of(&self, entrant_id: &str) -> Option<&Winner> {
        self.winners
            .iter()
            .rev()
            .find(|w| w.entrant_id == entrant_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(id: &str, item: &str, outcome: DrawOutcome) -> DrawInput {
        DrawInput {
            entrant_id: id.into(),
            display_name: format!("Name {id}"),
            item_name: item.into(),
            outcome,
        }
    }

    #[test]
    fn phases() {
        use Phase::*;
        let all = [
            Draft, Ready, Open, Closed, Frozen, Drawing, Finished, Purged,
        ];
        let over: Vec<_> = all.iter().filter(|p| p.is_over()).collect();
        let live: Vec<_> = all.iter().filter(|p| p.is_live()).collect();
        assert_eq!(over, [&Finished, &Purged]);
        assert_eq!(live, [&Ready, &Open, &Closed, &Frozen, &Drawing]);
        assert_eq!(serde_json::to_string(&Drawing).unwrap(), "\"drawing\"");
        assert_eq!(
            serde_json::to_string(&DrawOutcome::NotHere).unwrap(),
            "\"not_here\""
        );
    }

    #[test]
    fn winners_and_last() {
        let v = ParticipantView::new(
            7,
            Phase::Drawing,
            None,
            [
                draw("a", "Book", DrawOutcome::Claimed),
                draw("b", "Mug", DrawOutcome::Drawn),
                draw("c", "Shirt", DrawOutcome::NotHere),
                draw("d", "Shirt", DrawOutcome::Declined),
            ],
            3,
        );
        assert_eq!(
            v.winners,
            [
                Winner {
                    entrant_id: "a".into(),
                    item_name: "Book".into(),
                    claimed: true
                },
                Winner {
                    entrant_id: "b".into(),
                    item_name: "Mug".into(),
                    claimed: false
                },
            ]
        );
        assert_eq!(
            v.last,
            Some(LastDraw {
                display_name: "Name b".into(),
                item_name: "Mug".into()
            }),
            "the no-show and the decline are skipped"
        );
        assert_eq!(v.win_of("a").map(|w| w.claimed), Some(true));
        assert!(v.win_of("c").is_none());
    }

    #[test]
    fn no_winners_no_last() {
        let v = ParticipantView::new(
            1,
            Phase::Drawing,
            None,
            [draw("c", "Shirt", DrawOutcome::NotHere)],
            1,
        );
        assert!(v.winners.is_empty());
        assert_eq!(v.last, None);
    }

    #[test]
    fn same_content_ignores_v() {
        let a = ParticipantView::new(1, Phase::Open, None, [], 2);
        let mut b = a.clone();
        b.v = 9;
        assert!(a.same_content(&b));
        b.prizes_left = 1;
        assert!(!a.same_content(&b));
        let mut c = a.clone();
        c.winners.push(Winner {
            entrant_id: "x".into(),
            item_name: "y".into(),
            claimed: false,
        });
        assert!(!a.same_content(&c));
        let mut d = c.clone();
        d.winners[0].claimed = true;
        assert!(!c.same_content(&d), "a claim is a change");
    }

    #[test]
    fn json_is_flat_snake_case() {
        let v = ParticipantView::new(
            3,
            Phase::Drawing,
            Some(Current {
                item_name: "Mug".into(),
                round: 42,
                round_time: 1_700_000_000,
            }),
            [draw("a", "Book", DrawOutcome::Claimed)],
            1,
        );
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "v": 3,
                "state": "drawing",
                "current": { "item_name": "Mug", "round": 42, "round_time": 1_700_000_000 },
                "last": { "display_name": "Name a", "item_name": "Book" },
                "winners": [{ "entrant_id": "a", "item_name": "Book", "claimed": true }],
                "prizes_left": 1,
            })
        );
        let back: ParticipantView = serde_json::from_value(json).unwrap();
        assert_eq!(back, v);
    }
}
