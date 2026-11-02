//! The audit log: who did what to a raffle.

use rafi_lifecycle::RaffleState;

use crate::{Db, Result};

/// Who acts: the owner key the raffle belongs to (a Clerk organization, or
/// else the user), and the user acting for it. For a personal account the two
/// are the same; in an organization the user tells the members apart.
/// Housekeeping is [`HOUSEKEEPING`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor<'a> {
    pub owner: &'a str,
    pub user: &'a str,
}

impl<'a> Actor<'a> {
    /// A personal account, or a test's owner: the user is the owner.
    pub const fn solo(owner: &'a str) -> Self {
        Self { owner, user: owner }
    }
}

/// The automatic erasure's actor (`housekeeping`), in both columns.
pub const HOUSEKEEPING: Actor<'static> = Actor::solo("housekeeping");

/// What was done, as the audit log's `action` column has it. The strings are
/// written into the database, so a variant's string never changes; a new
/// kind of action is a new variant. `detail` says what it was about (IDs and
/// counts, never personal data).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A raffle created (or duplicated); detail: its title.
    Create,
    /// Settings changed; detail: the fields, by name.
    Settings,
    /// The registration lifecycle moved (publish, open, close, unpublish):
    /// the state it moved to.
    To(RaffleState),
    /// The raffle deleted; by housekeeping's owner-left removal, detail `owner left`.
    Delete,
    /// The list frozen and handed to the engine.
    Freeze,
    /// A prize drawn; detail: the item.
    Draw,
    /// A drawn item decided; detail: the item and the resolution.
    Resolve,
    /// An item withdrawn.
    Void,
    /// A stalled draw abandoned.
    Abandon,
    /// The raffle finished.
    Finish,
    /// A winner's prize marked delivered, or no longer; detail: the entrant.
    Deliver,
    Undeliver,
    /// An entrant's name or visibility changed; detail: the entrant.
    PatchEntrant,
    /// A prize's delivery changed; detail: the prize and the delivery.
    SetPrizeDelivery,
    /// People added by the organizer, or that add undone; detail: the batch.
    Add,
    UndoAdd,
    /// Fake entrants added or removed (test mode); detail: how many.
    AddFakes,
    RemoveFakes,
    /// An entrant removed (at their request or not), or their data erased.
    RemoveEntrant,
    EraseEntrant,
    /// Someone the organizer added took themselves off, or that was undone.
    Object,
    Unobject,
    /// The raffle's personal data erased early by the organizer, or (detail
    /// `owner left`) because they removed their data.
    Purge,
    /// Every prize marked delivered.
    DeliverAll,
    /// The live channel ended.
    EndLive,
    /// The winners exported as CSV.
    ExportCsv,
    /// Housekeeping: a raffle finished at the ceiling, entrants erased on
    /// their date (detail: how many), a raffle purged at the ceiling, a raffle
    /// never frozen expired (detail: how many entrants).
    AutoFinish,
    AutoErase,
    AutoPurge,
    Expire,
}

impl Action {
    /// Every action whose string doesn't depend on a state, and the states
    /// [`To`](Self::To) can name: every string the log can hold.
    pub const FIXED: [Self; 29] = [
        Self::Create,
        Self::Settings,
        Self::Delete,
        Self::Freeze,
        Self::Draw,
        Self::Resolve,
        Self::Void,
        Self::Abandon,
        Self::Finish,
        Self::Deliver,
        Self::Undeliver,
        Self::PatchEntrant,
        Self::SetPrizeDelivery,
        Self::Add,
        Self::UndoAdd,
        Self::AddFakes,
        Self::RemoveFakes,
        Self::RemoveEntrant,
        Self::EraseEntrant,
        Self::Object,
        Self::Unobject,
        Self::Purge,
        Self::DeliverAll,
        Self::EndLive,
        Self::ExportCsv,
        Self::AutoFinish,
        Self::AutoErase,
        Self::AutoPurge,
        Self::Expire,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Settings => "settings",
            Self::To(state) => state.as_str(),
            Self::Delete => "delete",
            Self::Freeze => "freeze",
            Self::Draw => "draw",
            Self::Resolve => "resolve",
            Self::Void => "void",
            Self::Abandon => "abandon",
            Self::Finish => "finish",
            Self::Deliver => "deliver",
            Self::Undeliver => "undeliver",
            Self::PatchEntrant => "patch_entrant",
            Self::SetPrizeDelivery => "set_prize_delivery",
            Self::Add => "add",
            Self::UndoAdd => "undo_add",
            Self::AddFakes => "add_fakes",
            Self::RemoveFakes => "remove_fakes",
            Self::RemoveEntrant => "remove_entrant",
            Self::EraseEntrant => "erase_entrant",
            Self::Object => "object",
            Self::Unobject => "unobject",
            Self::Purge => "purge",
            Self::DeliverAll => "deliver_all",
            Self::EndLive => "end_live",
            Self::ExportCsv => "export_csv",
            Self::AutoFinish => "auto_finish",
            Self::AutoErase => "auto_erase",
            Self::AutoPurge => "auto_purge",
            Self::Expire => "expire",
        }
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Records that `actor` did `action` (with `detail`) to the raffle at `now`.
pub async fn record<'a>(
    db: &Db,
    raffle_id: &str,
    actor: impl Into<Actor<'a>>,
    action: Action,
    detail: &str,
    now: i64,
) -> Result<()> {
    let actor = actor.into();
    db.query(
        "INSERT INTO audit (raffle_id, at, actor, user_id, action, detail) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(raffle_id)
    .bind(now)
    .bind(actor.owner)
    .bind(actor.user)
    .bind(action.as_str())
    .bind(detail)
    .run()
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every string is distinct, snake_case, and none is a state's (which
    /// [`Action::To`] writes).
    #[test]
    fn the_strings_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for a in Action::FIXED {
            let s = a.as_str();
            assert!(s.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{s}");
            assert!(seen.insert(s), "{s} twice");
            assert!(
                RaffleState::ALL.iter().all(|state| state.as_str() != s),
                "{s} is a state's"
            );
        }
    }
}
