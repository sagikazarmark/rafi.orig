//! How a prize reaches its winner ([`Delivery`]), and what that means for
//! the claim.

use serde::{Deserialize, Serialize};

/// How a prize reaches its winner: chosen per prize by the organizer, and
/// changeable until the raffle is over
/// ([`prize_delivery_editable`](crate::RaffleState::prize_delivery_editable)):
/// it says nothing about who wins, so it isn't in the manifest.
///
/// It decides whether claiming one of its items also marks the winner's
/// prize delivered ([`delivered_on_claim`](Self::delivered_on_claim)), which
/// is what moves a winner off the winner hold onto the non-winners' erasure
/// clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// Sent to the winner after the raffle (a voucher by email, a parcel):
    /// the organizer marks it delivered once it reached them. The default:
    /// taken for a prize handed over on the spot, it only keeps the
    /// winner's data until it is marked delivered (at the latest, the
    /// winner hold); the other way round, it would erase the address a
    /// voucher is still to be sent to.
    #[default]
    Later,
    /// Handed over on the spot, when the winner claims it with their code.
    OnTheSpot,
}

impl Delivery {
    /// Every way, the default first.
    pub const ALL: [Self; 2] = [Self::Later, Self::OnTheSpot];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Later => "later",
            Self::OnTheSpot => "on_the_spot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "later" => Self::Later,
            "on_the_spot" => Self::OnTheSpot,
            _ => return None,
        })
    }

    /// Whether claiming the prize delivers it: the winner leaves with it.
    /// Otherwise the organizer marks it delivered when it reaches them.
    pub fn delivered_on_claim(self) -> bool {
        self == Self::OnTheSpot
    }
}

/// What a claim does to its winner's delivery, which is one mark per
/// entrant ([`raffle::on_claim`](crate::raffle::on_claim)): a winner counts
/// as delivered when every prize they claimed is. With more items than
/// people, one entrant can claim two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnClaim {
    /// Mark them delivered (if they aren't): the prize was handed over on
    /// the spot, and so was every other one they claimed.
    Deliver,
    /// Mark them not delivered (if they are): they are owed a prize sent
    /// later, whatever an earlier one said.
    Undeliver,
    /// Leave it: a prize handed over, but another still to be sent; or a
    /// prize that isn't known.
    Nothing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_names() {
        for d in Delivery::ALL {
            assert_eq!(Delivery::parse(d.as_str()), Some(d));
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(json, format!("\"{}\"", d.as_str()), "serde and SQL agree");
        }
        assert_eq!(Delivery::parse("handed_over"), None);
    }

    #[test]
    fn only_a_prize_handed_over_on_the_spot_is_delivered_by_its_claim() {
        assert!(Delivery::OnTheSpot.delivered_on_claim());
        assert!(!Delivery::Later.delivered_on_claim());
        assert_eq!(
            Delivery::default(),
            Delivery::Later,
            "a prize nobody said anything about keeps its winner's data"
        );
    }
}
