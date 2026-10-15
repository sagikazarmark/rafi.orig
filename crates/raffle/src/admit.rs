//! Whether the raffle would take a command now, while a draw may be pending.

use crate::{Command, Decision, Error, Raffle, Resolution};

impl Raffle {
    /// Checks `command` the way an engine applies it: a dry run of the rules
    /// on a copy; `self` doesn't change.
    ///
    /// Engines apply one command at a time. While a draw waits for its round,
    /// an engine queues a decision behind it and applies it to the settled
    /// state; only [`Resolution::Claimed`] is admitted then (it can't change
    /// who the draw picks). The rest waits for the draw to settle or stall.
    ///
    /// Callers check before sending, for an immediate answer: a draw is
    /// accepted asynchronously, so a draw the engine refuses goes unnoticed
    /// (only the stored raffle shows progress).
    pub fn admit(&self, command: &Command) -> Result<(), Error> {
        let mut dry = self.clone();
        match command {
            // Drawing the pending item again retries its committed round;
            // nothing else may start meanwhile.
            Command::Draw(request) => {
                let source = *dry.beacon();
                dry.announce(request, &source, 0, 0).map(|_| ())
            }
            Command::Decide(
                decision @ Decision::Resolve {
                    resolution: Resolution::Claimed,
                    ..
                },
            ) => dry.decide(decision),
            Command::Decide(_) if dry.pending().is_some() && !dry.stalled() => {
                Err(Error::DrawInProgress)
            }
            Command::Decide(decision) => dry.decide(decision),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        BeaconSource, Command, Decision, DrawRequest, Error, Manifest, ManifestItem, Raffle,
        Resolution,
    };

    fn raffle() -> Raffle {
        let manifest = Manifest::new(
            "r1",
            BeaconSource::QUICKNET,
            vec!["a".into(), "b".into()],
            ["item-1", "item-2"]
                .map(|id| ManifestItem {
                    id: id.into(),
                    name: id.into(),
                })
                .into(),
        );
        Raffle::open(&manifest).unwrap()
    }

    /// A raffle whose draw of `item-1` waits for its round.
    fn pending() -> Raffle {
        let mut raffle = raffle();
        let source = *raffle.beacon();
        raffle
            .announce(
                &DrawRequest {
                    item: "item-1".into(),
                },
                &source,
                1,
                0,
            )
            .unwrap();
        raffle
    }

    fn draw(item: &str) -> Command {
        Command::Draw(DrawRequest { item: item.into() })
    }

    fn decide(decision: Decision) -> Command {
        Command::Decide(decision)
    }

    #[test]
    fn admits_what_the_rules_allow_and_changes_nothing() {
        let raffle = raffle();
        raffle.admit(&draw("item-1")).unwrap();
        raffle
            .admit(&decide(Decision::Void {
                item: "item-2".into(),
            }))
            .unwrap();
        assert_eq!(raffle, self::raffle());
    }

    #[test]
    fn refuses_what_the_rules_refuse() {
        let err = raffle().admit(&draw("nope")).unwrap_err();
        assert!(matches!(err, Error::UnknownItem(_)), "{err:?}");
    }

    #[test]
    fn retries_the_pending_draw_only() {
        let raffle = pending();
        raffle.admit(&draw("item-1")).unwrap();
        let err = raffle.admit(&draw("item-2")).unwrap_err();
        assert!(matches!(err, Error::DrawInProgress), "{err:?}");
    }

    #[test]
    fn holds_decisions_back_while_a_draw_waits_for_its_round() {
        let raffle = pending();
        for decision in [
            Decision::Void {
                item: "item-2".into(),
            },
            Decision::Abandon {
                item: "item-1".into(),
            },
            Decision::Finish,
        ] {
            let err = raffle.admit(&decide(decision.clone())).unwrap_err();
            assert!(
                matches!(err, Error::DrawInProgress),
                "{decision:?}: {err:?}"
            );
        }
    }

    #[test]
    fn lets_a_claim_through_while_a_draw_waits() {
        // Checked by the rules, not held back: item-2 has no winner yet.
        let claim = Decision::Resolve {
            item: "item-2".into(),
            resolution: Resolution::Claimed,
        };
        let err = pending().admit(&decide(claim)).unwrap_err();
        assert!(matches!(err, Error::ItemNotDrawn(_)), "{err:?}");
    }
}
