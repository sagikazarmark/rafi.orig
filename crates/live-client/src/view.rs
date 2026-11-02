//! The participant view: everything a phone needs, and nothing more.

use serde::{Deserialize, Serialize};

/// Display name shown for an entrant the organizer hid.
pub const HIDDEN_NAME: &str = "★ ★ ★";

/// How entrant `id` shows on public screens, given what is known of them:
/// their name (`Some(Some(name))`), [`HIDDEN_NAME`] if the organizer hid it
/// (`Some(None)`), or the start of their ID if nothing is (`None`: erased,
/// or not read).
pub fn public_name(id: &str, known: Option<Option<&str>>) -> String {
    match known {
        Some(Some(name)) => name.to_owned(),
        Some(None) => HIDDEN_NAME.to_owned(),
        None => format!("#{}", id.get(..6).unwrap_or(id)),
    }
}

/// A raffle's lifecycle state, as phones see it.
///
/// Mirrors `rafi_lifecycle::RaffleState` variant for variant, with the same
/// serde names. It lives here so the phone doesn't depend on `rafi-lifecycle`;
/// `rafi-live-server` maps one to the other with an exhaustive match, so
/// the two can't drift apart. [`Unknown`](Self::Unknown) is the one variant
/// the server never sends: it is what a phone running an older module reads
/// when a newer server adds a state, so the module keeps following instead
/// of freezing on its last view.
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
    /// A state this build doesn't know (the server is newer). Treated as
    /// live and not over: the phone keeps following, and shows the waiting
    /// card until a state it knows comes along.
    #[serde(other)]
    Unknown,
}

impl Phase {
    /// Whether the raffle has ended: nothing a phone shows changes after this.
    pub fn is_over(self) -> bool {
        matches!(self, Self::Finished | Self::Purged)
    }

    /// Whether the raffle is public and not over, which is when a page
    /// subscribes to [`ParticipantChannel`](crate::ParticipantChannel). Ready is included so a
    /// visitor's page can reload when registration opens, and an unknown
    /// state so an older module doesn't stop following a newer server.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Ready | Self::Open | Self::Closed | Self::Frozen | Self::Drawing | Self::Unknown
        )
    }
}

/// What a phone needs, and nothing more.
///
/// The Durable Object builds it from one consistent read of the database (see
/// [`ParticipantView::new`]) and publishes it on [`ParticipantChannel`](crate::ParticipantChannel);
/// the server also embeds it in the participant page. It has no time-dependent
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
    /// The prizes held (drawn or claimed), one entry each, so a phone can
    /// tell it won and what: an entrant pass 2 drew again (more prizes than
    /// people) is in it once per prize ([`wins_of`](Self::wins_of)).
    pub winners: Vec<Winner>,
    /// Prizes still to be won.
    pub prizes_left: u32,
}

/// The draw in progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Current {
    pub prize_name: String,
    /// The committed drand round.
    pub round: u64,
    /// Unix seconds the drand round is published.
    pub round_time: u64,
}

/// The latest draw, as shown between draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastDraw {
    pub display_name: String,
    pub prize_name: String,
    /// The winner has no phone to tell them they won (the organizer added
    /// them): the projector, which takes the reveal from here when the
    /// live channel is ahead, calls them to the front. Absent from an older
    /// server's view.
    #[serde(default)]
    pub no_phone: bool,
}

/// A prize held, and by whom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Winner {
    pub entrant_id: String,
    pub prize_name: String,
    /// The winner claimed the prize: they were there and the organizer
    /// checked their code. Not necessarily in their hands
    /// ([`sent_later`](Self::sent_later)).
    pub claimed: bool,
    /// The prize is sent to the winner after the raffle instead of being
    /// handed over on the spot. Absent from an older server's view.
    #[serde(default)]
    pub sent_later: bool,
}

/// How a drawn entrant's turn ended. Mirrors `rafi_raffle::Outcome` (mapped
/// exhaustively by `rafi-live-server`).
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
    pub prize_name: String,
    pub outcome: DrawOutcome,
    /// The prize is sent to the winner after the raffle.
    pub sent_later: bool,
    /// The winner has no phone to tell them they won.
    pub no_phone: bool,
}

impl ParticipantView {
    /// Builds the view. `draws` are the raffle's draws in chronological order.
    ///
    /// - `winners` are the draws that hold their prize, with `claimed` set for
    ///   claimed ones and `sent_later` as the draw says;
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
                prize_name: d.prize_name.clone(),
                claimed: d.outcome == DrawOutcome::Claimed,
                sent_later: d.sent_later,
            });
            last = Some(LastDraw {
                display_name: d.display_name,
                prize_name: d.prize_name,
                no_phone: d.no_phone,
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

    /// Every prize `entrant_id` holds, in the order they were drawn. More
    /// than one when there were more prizes than people: pass 2 draws the
    /// earlier winners again.
    pub fn wins_of(&self, entrant_id: &str) -> Vec<&Winner> {
        self.winners
            .iter()
            .filter(|w| w.entrant_id == entrant_id)
            .collect()
    }

    /// Whether `entrant_id` holds a prize here they didn't in `before`: a
    /// win to announce, the first or one more. Prizes are told apart by
    /// name, counted, so a second copy of the same prize is a new win too.
    pub fn won_since(&self, before: &Self, entrant_id: &str) -> bool {
        let before = before.wins_of(entrant_id);
        let now = self.wins_of(entrant_id);
        let count =
            |wins: &[&Winner], name: &str| wins.iter().filter(|w| w.prize_name == name).count();
        now.iter()
            .any(|w| count(&now, &w.prize_name) > count(&before, &w.prize_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_names() {
        assert_eq!(public_name("abcdef12", Some(Some("Anna"))), "Anna");
        assert_eq!(public_name("abcdef12", Some(None)), HIDDEN_NAME);
        assert_eq!(public_name("abcdef12", None), "#abcdef");
        assert_eq!(public_name("ab", None), "#ab");
    }

    fn draw(id: &str, prize: &str, outcome: DrawOutcome) -> DrawInput {
        DrawInput {
            entrant_id: id.into(),
            display_name: format!("Name {id}"),
            prize_name: prize.into(),
            outcome,
            sent_later: false,
            no_phone: false,
        }
    }

    #[test]
    fn phases() {
        use Phase::*;
        let all = [
            Draft, Ready, Open, Closed, Frozen, Drawing, Finished, Purged, Unknown,
        ];
        let over: Vec<_> = all.iter().filter(|p| p.is_over()).collect();
        let live: Vec<_> = all.iter().filter(|p| p.is_live()).collect();
        assert_eq!(over, [&Finished, &Purged]);
        assert_eq!(live, [&Ready, &Open, &Closed, &Frozen, &Drawing, &Unknown]);
        assert_eq!(serde_json::to_string(&Drawing).unwrap(), "\"drawing\"");
        // A state from a newer server: the module keeps following.
        let newer: Phase = serde_json::from_str("\"archived\"").unwrap();
        assert_eq!(newer, Unknown);
        let view: ParticipantView = serde_json::from_value(serde_json::json!({
            "v": 7, "state": "archived", "current": null, "last": null,
            "winners": [], "prizes_left": 1, "from_the_future": true
        }))
        .unwrap();
        assert!(view.state.is_live() && !view.state.is_over());
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
                DrawInput {
                    sent_later: true,
                    no_phone: true,
                    ..draw("b", "Mug", DrawOutcome::Drawn)
                },
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
                    prize_name: "Book".into(),
                    claimed: true,
                    sent_later: false,
                },
                Winner {
                    entrant_id: "b".into(),
                    prize_name: "Mug".into(),
                    claimed: false,
                    sent_later: true,
                },
            ]
        );
        assert_eq!(
            v.last,
            Some(LastDraw {
                display_name: "Name b".into(),
                prize_name: "Mug".into(),
                no_phone: true,
            }),
            "the no-show and the decline are skipped"
        );
        let claimed: Vec<_> = v.wins_of("a").iter().map(|w| w.claimed).collect();
        assert_eq!(claimed, [true]);
        assert!(v.wins_of("c").is_empty());
    }

    /// More prizes than people: pass 2 draws "a" again, and the view keeps
    /// both of their prizes, each with its own state, in draw order.
    #[test]
    fn a_winner_of_two_holds_both() {
        let v = ParticipantView::new(
            9,
            Phase::Drawing,
            None,
            [
                draw("a", "Book", DrawOutcome::Claimed),
                draw("b", "Mug", DrawOutcome::Claimed),
                DrawInput {
                    sent_later: true,
                    ..draw("a", "Voucher", DrawOutcome::Drawn)
                },
                draw("b", "Shirt", DrawOutcome::NotHere),
            ],
            1,
        );
        let wins: Vec<_> = v
            .wins_of("a")
            .iter()
            .map(|w| (w.prize_name.as_str(), w.claimed, w.sent_later))
            .collect();
        assert_eq!(wins, [("Book", true, false), ("Voucher", false, true)]);
        let wins: Vec<_> = v.wins_of("b").iter().map(|w| &w.prize_name).collect();
        assert_eq!(wins, ["Mug"], "a forfeited pass-2 draw isn't held");
    }

    #[test]
    fn a_win_is_new_if_its_prize_wasnt_held() {
        let view = |draws: Vec<DrawInput>| ParticipantView::new(1, Phase::Drawing, None, draws, 1);
        let none = view(vec![]);
        let one = view(vec![draw("a", "Book", DrawOutcome::Drawn)]);
        let claimed = view(vec![draw("a", "Book", DrawOutcome::Claimed)]);
        let two = view(vec![
            draw("a", "Book", DrawOutcome::Claimed),
            draw("b", "Mug", DrawOutcome::Claimed),
            draw("a", "Voucher", DrawOutcome::Drawn),
        ]);
        let twice = view(vec![
            draw("a", "Book", DrawOutcome::Claimed),
            draw("a", "Book", DrawOutcome::Drawn),
        ]);
        assert!(one.won_since(&none, "a"), "the first win");
        assert!(!one.won_since(&none, "b"), "someone else's");
        assert!(!claimed.won_since(&one, "a"), "a claim isn't a new win");
        assert!(two.won_since(&claimed, "a"), "a second prize is");
        assert!(!two.won_since(&two, "a"));
        assert!(twice.won_since(&claimed, "a"), "so is a second copy of one");
        assert!(!none.won_since(&two, "a"), "losing one isn't");
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
            prize_name: "y".into(),
            claimed: false,
            sent_later: false,
        });
        assert!(!a.same_content(&c));
        let mut d = c.clone();
        d.winners[0].claimed = true;
        assert!(!c.same_content(&d), "a claim is a change");
        let mut e = d.clone();
        e.winners[0].sent_later = true;
        assert!(
            !d.same_content(&e),
            "how the prize is delivered is a change"
        );
    }

    #[test]
    fn json_is_flat_snake_case() {
        let v = ParticipantView::new(
            3,
            Phase::Drawing,
            Some(Current {
                prize_name: "Mug".into(),
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
                "current": { "prize_name": "Mug", "round": 42, "round_time": 1_700_000_000 },
                "last": { "display_name": "Name a", "prize_name": "Book", "no_phone": false },
                "winners": [{ "entrant_id": "a", "prize_name": "Book", "claimed": true, "sent_later": false }],
                "prizes_left": 1,
            })
        );
        let mut json = json;
        let back: ParticipantView = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, v);
        // An older server's: no `no_phone`, read as a phone telling them.
        json["last"].as_object_mut().unwrap().remove("no_phone");
        let older: ParticipantView = serde_json::from_value(json).unwrap();
        assert_eq!(older, v);
    }

    #[test]
    fn an_older_servers_winner_is_not_sent_later() {
        let w: Winner = serde_json::from_value(serde_json::json!({
            "entrant_id": "a", "prize_name": "Book", "claimed": true
        }))
        .unwrap();
        assert!(w.claimed && !w.sent_later);
    }
}
