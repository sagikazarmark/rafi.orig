//! What phones see of a raffle ([`Snapshot`]) and the rules of its live
//! updates. The store reads a snapshot in one consistent batch
//! (`rafi_store::live::snapshot`); the servers that show phones a raffle
//! build the participant view from it (`rafi_live_server::raffle::view`).

use std::collections::HashMap;

use rafi_raffle::Raffle;

use crate::RaffleState;

/// The raffle as phones see it, read in one snapshot.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub state: RaffleState,
    /// Bumped by every write that changes what phones see.
    pub version: u64,
    pub live_ended_at: Option<i64>,
    /// The raffle as drawn, once the engine has opened it.
    pub drawn: Option<Raffle>,
    /// The display names of the drawn entrants only, by ID: no other name
    /// leaves the database. `None` for one the organizer hid: that name isn't
    /// read either.
    pub names: HashMap<String, Option<String>>,
    /// The prizes' total quantity (what is left before the raffle opens).
    pub prize_qty: u32,
}

impl Snapshot {
    /// Whether the live channel still runs: live updates haven't been ended.
    pub fn channel_open(&self) -> bool {
        self.live_ended_at.is_none()
    }

    /// Whether phones should hold a socket: the channel is open and the raffle
    /// is public and not over.
    pub fn accepts_sockets(&self) -> bool {
        self.channel_open() && self.state.is_live()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(state: RaffleState, live_ended_at: Option<i64>) -> Snapshot {
        Snapshot {
            state,
            version: 1,
            live_ended_at,
            drawn: None,
            names: HashMap::new(),
            prize_qty: 0,
        }
    }

    #[test]
    fn sockets_while_live_and_not_ended() {
        for state in RaffleState::ALL {
            assert_eq!(snapshot(state, None).accepts_sockets(), state.is_live());
            let ended = snapshot(state, Some(1));
            assert!(!ended.channel_open());
            assert!(!ended.accepts_sockets());
        }
    }
}
