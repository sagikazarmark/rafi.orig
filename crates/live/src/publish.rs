//! When a live channel publishes a freshly built participant view.

use crate::ParticipantView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Newer and different: send it to every socket.
    Publish,
    /// Newer, but phones would see the same thing (a join, say).
    Unchanged,
    /// Built from an older read than one already seen, or the same one again.
    Stale { seen: u64, got: u64 },
    /// The channel is closed (live updates ended, or the object is destroyed).
    Ignore,
}

/// `seen` is the highest version the channel has seen, `last` the view it
/// published last.
pub fn decide(
    open: bool,
    seen: u64,
    last: Option<&ParticipantView>,
    view: &ParticipantView,
) -> Decision {
    if !open {
        Decision::Ignore
    } else if view.v <= seen {
        Decision::Stale { seen, got: view.v }
    } else if last.is_some_and(|l| l.same_content(view)) {
        Decision::Unchanged
    } else {
        Decision::Publish
    }
}

#[cfg(test)]
mod tests {
    use crate::Phase;

    use super::*;

    fn view(v: u64, state: Phase) -> ParticipantView {
        ParticipantView::new(v, state, None, [], 2)
    }

    #[test]
    fn publishes_newer_different_views() {
        assert_eq!(
            decide(true, 0, None, &view(1, Phase::Open)),
            Decision::Publish
        );
        let last = view(3, Phase::Open);
        assert_eq!(
            decide(true, 3, Some(&last), &view(4, Phase::Frozen)),
            Decision::Publish
        );
    }

    #[test]
    fn skips_same_content() {
        let last = view(3, Phase::Open);
        assert_eq!(
            decide(true, 3, Some(&last), &view(9, Phase::Open)),
            Decision::Unchanged
        );
    }

    #[test]
    fn rejects_stale_and_duplicate_reads() {
        let last = view(5, Phase::Open);
        assert_eq!(
            decide(true, 5, Some(&last), &view(4, Phase::Frozen)),
            Decision::Stale { seen: 5, got: 4 }
        );
        assert_eq!(
            decide(true, 5, Some(&last), &view(5, Phase::Frozen)),
            Decision::Stale { seen: 5, got: 5 }
        );
    }

    #[test]
    fn ignores_everything_once_closed() {
        assert_eq!(
            decide(false, 0, None, &view(1, Phase::Open)),
            Decision::Ignore
        );
    }
}
