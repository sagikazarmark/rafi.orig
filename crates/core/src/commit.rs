//! The commit document published when the entrant list is frozen.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Hash, drand::ChainHash, draw::ALGO, sha256, state::DrawError};

/// A prize unit as committed at freeze time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRef {
    pub unit_id: String,
    pub item_id: String,
    pub item_name: String,
}

/// The ID of the `n`th unit (from 1) of prize item `item_id`.
pub fn unit_id(item_id: &str, n: u64) -> String {
    format!("{item_id}-{n}")
}

/// Expands prize items `(item_id, item_name, quantity)` into the units that get
/// committed and drawn one by one, in order.
pub fn units<'a>(items: impl IntoIterator<Item = (&'a str, &'a str, u64)>) -> Vec<UnitRef> {
    items
        .into_iter()
        .flat_map(|(item_id, item_name, qty)| {
            (1..=qty).map(move |n| UnitRef {
                unit_id: unit_id(item_id, n),
                item_id: item_id.to_owned(),
                item_name: item_name.to_owned(),
            })
        })
        .collect()
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

    /// The hex list hash of this document.
    pub fn list_hash_hex(&self) -> String {
        hex::encode(list_hash(&self.canonical_json()))
    }

    /// The idempotency key for freezing this document: the same list always
    /// freezes once.
    pub fn freeze_key(&self) -> String {
        format!("freeze-{}", self.list_hash_hex())
    }

    /// Checks that the document is one [`CommitDoc::new`] could have built for
    /// a drawable raffle: the current algorithm, at least one entrant and one
    /// unit, entrant IDs in canonical form (sorted, deduplicated, lowercase, so
    /// re-serializing it reproduces the published list hash), and unique unit
    /// IDs.
    pub fn validate(&self) -> Result<(), DrawError> {
        let invalid = |why: &str| Err(DrawError::InvalidCommit(why.to_owned()));
        if self.algo != ALGO {
            return invalid("unknown draw algorithm");
        }
        if self.entrant_ids.is_empty() {
            return invalid("nobody has entered");
        }
        if self.units.is_empty() {
            return invalid("there are no prizes");
        }
        if self
            .entrant_ids
            .iter()
            .any(|id| *id != id.to_ascii_lowercase())
            || self.entrant_ids.windows(2).any(|w| w[0] >= w[1])
        {
            return invalid("entrant IDs are not sorted, unique and lowercase");
        }
        let mut seen = HashSet::new();
        if !self.units.iter().all(|u| seen.insert(u.unit_id.as_str())) {
            return invalid("duplicate prize unit IDs");
        }
        Ok(())
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

    fn unit(id: &str) -> UnitRef {
        UnitRef {
            unit_id: id.into(),
            item_id: "i1".into(),
            item_name: "Ticket".into(),
        }
    }

    #[test]
    fn units_expand_items_in_order() {
        let units = units([("a", "Book", 2), ("b", "Mug", 0), ("c", "Pen", 1)]);
        let ids: Vec<_> = units.iter().map(|u| u.unit_id.as_str()).collect();
        assert_eq!(ids, ["a-1", "a-2", "c-1"]);
        assert_eq!(units[2].item_name, "Pen");
    }

    #[test]
    fn documents_built_by_new_validate() {
        let doc = CommitDoc::new(
            "r1",
            &QUICKNET,
            vec!["B".into(), "a".into()],
            vec![unit("u1")],
        );
        assert_eq!(doc.validate(), Ok(()));
        assert!(doc.freeze_key().starts_with("freeze-"));
        assert_eq!(doc.freeze_key(), format!("freeze-{}", doc.list_hash_hex()));
    }

    #[test]
    fn non_canonical_or_empty_documents_are_invalid() {
        let good = CommitDoc::new(
            "r1",
            &QUICKNET,
            vec!["a".into(), "b".into()],
            vec![unit("u1")],
        );
        let invalid = |doc: CommitDoc| matches!(doc.validate(), Err(DrawError::InvalidCommit(_)));

        assert!(invalid(CommitDoc {
            algo: "other".into(),
            ..good.clone()
        }));
        assert!(invalid(CommitDoc {
            entrant_ids: vec![],
            ..good.clone()
        }));
        assert!(invalid(CommitDoc {
            units: vec![],
            ..good.clone()
        }));
        assert!(invalid(CommitDoc {
            entrant_ids: vec!["b".into(), "a".into()],
            ..good.clone()
        }));
        assert!(invalid(CommitDoc {
            entrant_ids: vec!["a".into(), "a".into()],
            ..good.clone()
        }));
        assert!(invalid(CommitDoc {
            entrant_ids: vec!["A".into()],
            ..good.clone()
        }));
        assert!(invalid(CommitDoc {
            units: vec![unit("u1"), unit("u1")],
            ..good
        }));
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
