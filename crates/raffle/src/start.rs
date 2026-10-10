//! Starting the draw: freezing the entrant list, writing the drawing's
//! manifest from it, and handing that to the drawing engine. (The freeze
//! fixes the entrants and the prizes; "commit" is the drawing's word, for a
//! draw committing to a drand round.)
//!
//! The store has no interactive transactions, so this is a protocol of
//! separate writes, each guarded by the state it expects and safe to repeat:
//! close registration (the raffle freezes), read the now-fixed entrant list,
//! store the manifest (or [unfreeze](crate::RaffleState::unfreeze) when
//! nobody is left), open the drawing. [`next`] says which step a raffle is
//! at, so pressing "start" again picks up where an attempt stopped.

use crate::RaffleState;

/// The step starting the draw takes next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Freeze the raffle (from `from`), then
    /// [`WriteManifest`](Self::WriteManifest) with `unfreeze_to: from`. Needs
    /// entrants and prizes ([`ready`]).
    Freeze { from: RaffleState },
    /// Frozen without a manifest (a freeze stopped between its steps):
    /// build the manifest from the fixed entrant list and store it, or go
    /// back to `unfreeze_to` if nobody is left.
    WriteManifest { unfreeze_to: RaffleState },
    /// The manifest is stored, but the engine hasn't opened the drawing: hand
    /// it the stored manifest again (opening is idempotent).
    Resend,
    /// The engine has the drawing: nothing left to do.
    Started,
}

/// Why the draw can't start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not in a state the draw starts from.
    State(RaffleState),
    /// Nobody to draw from.
    NoEntrants,
    /// Nothing to draw for.
    NoPrizes,
}

/// The next step for a raffle in `state`, with or without a stored manifest
/// (`has_manifest`) and drawing (`opened`).
pub fn next(state: RaffleState, has_manifest: bool, opened: bool) -> Result<Start, Refusal> {
    use crate::Command;
    match state {
        from if from.apply(Command::Freeze).is_some() => Ok(Start::Freeze { from }),
        // Whatever it froze from, it may reopen from closed.
        RaffleState::Frozen if !opened && !has_manifest => Ok(Start::WriteManifest {
            unfreeze_to: RaffleState::Closed,
        }),
        RaffleState::Frozen if !opened => Ok(Start::Resend),
        RaffleState::Frozen | RaffleState::Drawing => Ok(Start::Started),
        s => Err(Refusal::State(s)),
    }
}

/// Whether a raffle with `entrants` active entrants and `items` drawing
/// items (prize copies) has a drawing to start.
pub fn ready(entrants: usize, items: usize) -> Result<(), Refusal> {
    if items == 0 {
        return Err(Refusal::NoPrizes);
    }
    if entrants == 0 {
        return Err(Refusal::NoEntrants);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use RaffleState as S;

    #[test]
    fn freezes_from_open_or_closed() {
        assert_eq!(
            next(S::Open, false, false),
            Ok(Start::Freeze { from: S::Open })
        );
        assert_eq!(
            next(S::Closed, false, false),
            Ok(Start::Freeze { from: S::Closed })
        );
    }

    #[test]
    fn picks_up_where_an_attempt_stopped() {
        assert_eq!(
            next(S::Frozen, false, false),
            Ok(Start::WriteManifest {
                unfreeze_to: S::Closed
            })
        );
        assert_eq!(next(S::Frozen, true, false), Ok(Start::Resend));
        assert_eq!(next(S::Frozen, true, true), Ok(Start::Started));
        assert_eq!(next(S::Drawing, true, true), Ok(Start::Started));
    }

    /// Every state, with and without a manifest and a drawing: the step, or
    /// the refusal.
    #[test]
    fn every_combination() {
        for s in S::ALL {
            for has_manifest in [false, true] {
                for opened in [false, true] {
                    let got = next(s, has_manifest, opened);
                    let want = match s {
                        S::Open | S::Closed => Ok(Start::Freeze { from: s }),
                        S::Frozen if opened => Ok(Start::Started),
                        S::Frozen if has_manifest => Ok(Start::Resend),
                        S::Frozen => Ok(Start::WriteManifest {
                            unfreeze_to: S::Closed,
                        }),
                        S::Drawing => Ok(Start::Started),
                        _ => Err(Refusal::State(s)),
                    };
                    assert_eq!(got, want, "{s:?}, manifest {has_manifest}, opened {opened}");
                }
            }
        }
    }

    #[test]
    fn refuses_elsewhere() {
        for s in [S::Draft, S::Ready, S::Finished, S::Purged] {
            assert_eq!(next(s, false, false), Err(Refusal::State(s)));
        }
    }

    #[test]
    fn needs_prizes_then_entrants() {
        assert_eq!(ready(0, 0), Err(Refusal::NoPrizes));
        assert_eq!(ready(0, 1), Err(Refusal::NoEntrants));
        assert_eq!(ready(1, 1), Ok(()));
    }
}
