//! The commit document published when the entrant list is frozen.

use serde::{Deserialize, Serialize};

use crate::{Hash, drand::ChainHash, draw::ALGO, sha256};

/// A prize unit as committed at freeze time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRef {
    pub unit_id: String,
    pub item_id: String,
    pub item_name: String,
}

/// The frozen raffle: who is in, and what can be won.
///
/// Field order is fixed and the document contains no maps or floats, so
/// `serde_json` serialization is canonical. The exact serialized string is
/// published alongside the hash, so verifiers never need to re-serialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDoc {
    pub algo: String,
    pub raffle_id: String,
    pub drand_chain: String,
    /// Sorted, deduplicated, lowercase hex entrant IDs.
    pub entrant_ids: Vec<String>,
    pub units: Vec<UnitRef>,
}

impl CommitDoc {
    /// `drand_chain` is the chain whose beacons the draws will use.
    pub fn new(
        raffle_id: impl Into<String>,
        drand_chain: &ChainHash,
        mut entrant_ids: Vec<String>,
        units: Vec<UnitRef>,
    ) -> Self {
        for id in &mut entrant_ids {
            *id = id.to_ascii_lowercase();
        }
        entrant_ids.sort();
        entrant_ids.dedup();
        Self {
            algo: ALGO.to_owned(),
            raffle_id: raffle_id.into(),
            drand_chain: drand_chain.to_string(),
            entrant_ids,
            units,
        }
    }

    /// The canonical serialization that gets hashed and published.
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(self).expect("commit doc serializes")
    }
}

/// `list_hash = SHA256(canonical commit document)`.
pub fn list_hash(canonical_json: &str) -> Hash {
    sha256(&[canonical_json.as_bytes()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drand::chains::QUICKNET_HASH as QUICKNET;

    #[test]
    fn entrants_are_sorted_and_deduped() {
        let doc = CommitDoc::new(
            "r1",
            &QUICKNET,
            vec!["bb".into(), "AA".into(), "aa".into()],
            vec![],
        );
        assert_eq!(doc.entrant_ids, vec!["aa", "bb"]);
    }

    #[test]
    fn canonical_json_is_stable() {
        let doc = CommitDoc::new(
            "r1",
            &QUICKNET,
            vec!["01".into()],
            vec![UnitRef {
                unit_id: "u1".into(),
                item_id: "i1".into(),
                item_name: "Ticket".into(),
            }],
        );
        let json = doc.canonical_json();
        let parsed: CommitDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.canonical_json(), json);
        assert!(json.starts_with(r#"{"algo":"rafi-draw-v1","raffle_id":"r1""#));
    }
}
