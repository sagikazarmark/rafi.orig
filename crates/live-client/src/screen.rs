//! Which card a phone shows: a port of `render` in
//! `apps/participant/fallback.js`, as a pure function.

use crate::view::{LastDraw, ParticipantView, Phase};

/// The phone's live card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// A visitor's page predates registration opening: reload it to get the
    /// join form. The card renders nothing; the WASM reloads.
    Reload,
    /// This phone's entrant holds a prize.
    Won {
        prize_name: String,
        /// The organizer has handed the prize over.
        claimed: bool,
        /// The raffle is over.
        done: bool,
    },
    /// The raffle is over and this phone didn't win.
    Thanks,
    /// A draw is in progress.
    Countdown {
        prize_name: String,
        round: u64,
        /// Whole seconds until the round, rounded up; 0 once it's due.
        secs_left: u64,
    },
    /// Frozen or drawing, with no draw in progress.
    BetweenDraws {
        last: Option<LastDraw>,
        /// This phone has an entrant (it's in the draw).
        entered: bool,
        prizes_left: u32,
    },
    /// An entrant waiting for the draw to start.
    Waiting,
    /// Nothing to show (a visitor before the draw).
    Blank,
}

/// Picks the card for `view`.
///
/// - `me` is the entrant ID from the page's cookie, if any;
/// - `page_state` is the state the page was rendered in;
/// - `now_ms` is the server-synced clock, in Unix milliseconds.
pub fn screen(view: &ParticipantView, me: Option<&str>, page_state: Phase, now_ms: u64) -> Screen {
    if me.is_none() && view.state == Phase::Open && page_state != Phase::Open {
        return Screen::Reload;
    }
    if let Some(w) = me.and_then(|me| view.win_of(me)) {
        return Screen::Won {
            prize_name: w.prize_name.clone(),
            claimed: w.claimed,
            done: view.state.is_over(),
        };
    }
    if view.state.is_over() {
        return Screen::Thanks;
    }
    if let Some(c) = &view.current {
        return Screen::Countdown {
            prize_name: c.prize_name.clone(),
            round: c.round,
            secs_left: c
                .round_time
                .saturating_mul(1000)
                .saturating_sub(now_ms)
                .div_ceil(1000),
        };
    }
    if matches!(view.state, Phase::Frozen | Phase::Drawing) {
        return Screen::BetweenDraws {
            last: view.last.clone(),
            entered: me.is_some(),
            prizes_left: view.prizes_left,
        };
    }
    if me.is_some() {
        Screen::Waiting
    } else {
        Screen::Blank
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Current, DrawInput, DrawOutcome};

    const T: u64 = 1_700_000_000;

    fn draw(id: &str, prize: &str, outcome: DrawOutcome) -> DrawInput {
        DrawInput {
            entrant_id: id.into(),
            display_name: format!("Name {id}"),
            prize_name: prize.into(),
            outcome,
        }
    }

    fn view(state: Phase, current: Option<Current>, draws: Vec<DrawInput>) -> ParticipantView {
        ParticipantView::new(1, state, current, draws, 2)
    }

    fn current() -> Option<Current> {
        Some(Current {
            prize_name: "Mug".into(),
            round: 42,
            round_time: T,
        })
    }

    #[test]
    fn waiting_and_blank_before_the_draw() {
        for state in [Phase::Ready, Phase::Open, Phase::Closed] {
            let v = view(state, None, vec![]);
            assert_eq!(screen(&v, Some("a"), state, 0), Screen::Waiting);
            assert_eq!(screen(&v, None, state, 0), Screen::Blank);
        }
    }

    #[test]
    fn reload_when_registration_opens_after_page_load() {
        let v = view(Phase::Open, None, vec![]);
        assert_eq!(screen(&v, None, Phase::Ready, 0), Screen::Reload);
        assert_eq!(screen(&v, None, Phase::Closed, 0), Screen::Reload);
        // A page already showing the form, or an entrant, doesn't reload.
        assert_eq!(screen(&v, None, Phase::Open, 0), Screen::Blank);
        assert_eq!(screen(&v, Some("a"), Phase::Ready, 0), Screen::Waiting);
    }

    #[test]
    fn countdown_rounds_up_then_shows_due() {
        let v = view(Phase::Drawing, current(), vec![]);
        let at = |now_ms| screen(&v, Some("a"), Phase::Drawing, now_ms);
        let cd = |secs_left| Screen::Countdown {
            prize_name: "Mug".into(),
            round: 42,
            secs_left,
        };
        assert_eq!(at(T * 1000 - 3000), cd(3));
        assert_eq!(at(T * 1000 - 2001), cd(3));
        assert_eq!(at(T * 1000 - 1), cd(1));
        assert_eq!(at(T * 1000), cd(0), "due: the card shows …");
        assert_eq!(at(T * 1000 + 5000), cd(0));
        // Visitors see the countdown too.
        assert_eq!(screen(&v, None, Phase::Drawing, T * 1000 - 3000), cd(3));
    }

    #[test]
    fn won_drawn_claimed_and_done() {
        let draws = || {
            vec![
                draw("a", "Book", DrawOutcome::Drawn),
                draw("b", "Mug", DrawOutcome::Claimed),
            ]
        };
        let v = view(Phase::Drawing, None, draws());
        assert_eq!(
            screen(&v, Some("a"), Phase::Drawing, 0),
            Screen::Won {
                prize_name: "Book".into(),
                claimed: false,
                done: false
            }
        );
        assert_eq!(
            screen(&v, Some("b"), Phase::Drawing, 0),
            Screen::Won {
                prize_name: "Mug".into(),
                claimed: true,
                done: false
            },
            "a claimed winner isn't sent to the organizer"
        );
        // A win outranks a draw in progress for someone else.
        let v = view(Phase::Drawing, current(), draws());
        assert!(matches!(
            screen(&v, Some("a"), Phase::Drawing, 0),
            Screen::Won { .. }
        ));
        let v = view(Phase::Finished, None, draws());
        assert_eq!(
            screen(&v, Some("a"), Phase::Drawing, 0),
            Screen::Won {
                prize_name: "Book".into(),
                claimed: false,
                done: true
            }
        );
    }

    #[test]
    fn a_no_show_does_not_win() {
        let v = view(
            Phase::Drawing,
            None,
            vec![
                draw("a", "Book", DrawOutcome::Drawn),
                draw("c", "Mug", DrawOutcome::NotHere),
            ],
        );
        let last = Some(LastDraw {
            display_name: "Name a".into(),
            prize_name: "Book".into(),
        });
        assert_eq!(
            screen(&v, Some("c"), Phase::Drawing, 0),
            Screen::BetweenDraws {
                last: last.clone(),
                entered: true,
                prizes_left: 2
            },
            "the no-show isn't a winner, and `last` skips them"
        );
    }

    #[test]
    fn between_draws_for_entrants_and_visitors() {
        let v = view(Phase::Frozen, None, vec![]);
        assert_eq!(
            screen(&v, Some("x"), Phase::Frozen, 0),
            Screen::BetweenDraws {
                last: None,
                entered: true,
                prizes_left: 2
            }
        );
        let v = view(
            Phase::Drawing,
            None,
            vec![draw("a", "Book", DrawOutcome::Drawn)],
        );
        let last = Some(LastDraw {
            display_name: "Name a".into(),
            prize_name: "Book".into(),
        });
        assert_eq!(
            screen(&v, Some("x"), Phase::Drawing, 0),
            Screen::BetweenDraws {
                last: last.clone(),
                entered: true,
                prizes_left: 2
            }
        );
        assert_eq!(
            screen(&v, None, Phase::Drawing, 0),
            Screen::BetweenDraws {
                last,
                entered: false,
                prizes_left: 2
            }
        );
    }

    #[test]
    fn thanks_when_over() {
        for state in [Phase::Finished, Phase::Purged] {
            let v = view(state, None, vec![draw("a", "Book", DrawOutcome::Claimed)]);
            assert_eq!(screen(&v, Some("x"), state, 0), Screen::Thanks);
            assert_eq!(screen(&v, None, state, 0), Screen::Thanks);
        }
    }
}
