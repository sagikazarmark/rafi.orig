//! What phones see of a raffle ([`Snapshot`]) and the rules of its live
//! updates. The store reads a snapshot in one consistent batch
//! (`rafi_store::live::snapshot`); the servers that show phones a raffle
//! build the participant view from it (`rafi_live_server::raffle::view`).

use std::collections::HashMap;

use rafi_raffle::{ItemId, Raffle};

use crate::{RaffleState, prize::Delivery};

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
    /// How each prize reaches its winner, by prize ID: a winner's phone says
    /// whether the prize is still to be sent once they claimed it.
    pub deliveries: HashMap<String, Delivery>,
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

    /// How `item`'s prize reaches its winner, if the prize is known.
    pub fn delivery(&self, item: &ItemId) -> Option<Delivery> {
        self.deliveries.get(crate::raffle::prize_id(item)).copied()
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
            deliveries: HashMap::new(),
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

    #[test]
    fn an_items_delivery_is_its_prizes() {
        let mut s = snapshot(RaffleState::Drawing, None);
        s.deliveries = HashMap::from([
            ("p1".to_owned(), Delivery::OnTheSpot),
            ("p2".to_owned(), Delivery::Later),
        ]);
        let item = |id: &str| ItemId::from(id);
        assert_eq!(s.delivery(&item("p1-1")), Some(Delivery::OnTheSpot));
        assert_eq!(s.delivery(&item("p1-12")), Some(Delivery::OnTheSpot));
        assert_eq!(s.delivery(&item("p2-1")), Some(Delivery::Later));
        assert_eq!(s.delivery(&item("p3-1")), None, "no such prize");
    }
}
