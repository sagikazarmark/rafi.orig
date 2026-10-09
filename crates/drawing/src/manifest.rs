//! The manifest: everything that decides a drawing's outcome except the
//! randomness. Published (with its hash) when the drawing opens.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Hash, drand::ChainHash, draw::ALGO, error::Error, sha256};

/// An item as committed in the manifest: one thing one entrant can win.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRef {
    pub id: String,
    pub name: String,
}

/// Who is in, what can be won, and how the winners will be drawn.
///
/// Field order is fixed and the document contains no maps or floats, so
/// `serde_json` serialization is canonical. The exact serialized string is
/// published alongside the hash, so verifiers never need to re-serialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub algo: String,
    pub raffle_id: String,
    pub drand_chain: String,
    /// Sorted, deduplicated, lowercase hex entrant IDs.
    pub entrant_ids: Vec<String>,
    pub items: Vec<ItemRef>,
}

impl Manifest {
    /// `drand_chain` is the chain whose beacons the draws will use.
    pub fn new(
        raffle_id: impl Into<String>,
        drand_chain: &ChainHash,
        mut entrant_ids: Vec<String>,
        items: Vec<ItemRef>,
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
            items,
        }
    }

    /// The canonical serialization that gets hashed and published.
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(self).expect("manifest serializes")
    }

    /// The hex hash of this manifest.
    pub fn hash_hex(&self) -> String {
        hex::encode(hash(&self.canonical_json()))
    }

    /// The idempotency key for opening a drawing from this manifest: the same
    /// manifest opens once.
    pub fn open_key(&self) -> String {
        format!("open-{}", self.hash_hex())
    }

    /// Checks that the document is one [`Manifest::new`] could have built for
    /// a drawing that can be drawn: the current algorithm, at least one entrant and one
    /// item, entrant IDs in canonical form (sorted, deduplicated, lowercase, so
    /// re-serializing it reproduces the published manifest hash), and unique item
    /// IDs.
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |why: &str| Err(Error::InvalidManifest(why.to_owned()));
        if self.algo != ALGO {
            return invalid("unknown draw algorithm");
        }
        if self.entrant_ids.is_empty() {
            return invalid("nobody has entered");
        }
        if self.items.is_empty() {
            return invalid("there are no items");
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
        if !self.items.iter().all(|u| seen.insert(u.id.as_str())) {
            return invalid("duplicate item IDs");
        }
        Ok(())
    }
}

/// `manifest_hash = SHA256(canonical manifest)`.
pub fn hash(canonical_json: &str) -> Hash {
    sha256(&[canonical_json.as_bytes()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drand::chains::QUICKNET_HASH as QUICKNET;

    #[test]
    fn entrants_are_sorted_and_deduped() {
        let doc = Manifest::new(
            "r1",
            &QUICKNET,
            vec!["bb".into(), "AA".into(), "aa".into()],
            vec![],
        );
        assert_eq!(doc.entrant_ids, vec!["aa", "bb"]);
    }

    fn item(id: &str) -> ItemRef {
        ItemRef {
            id: id.into(),
            name: "Ticket".into(),
        }
    }

    #[test]
    fn documents_built_by_new_validate() {
        let doc = Manifest::new(
            "r1",
            &QUICKNET,
            vec!["B".into(), "a".into()],
            vec![item("u1")],
        );
        assert_eq!(doc.validate(), Ok(()));
        assert!(doc.open_key().starts_with("open-"));
        assert_eq!(doc.open_key(), format!("open-{}", doc.hash_hex()));
    }

    #[test]
    fn non_canonical_or_empty_documents_are_invalid() {
        let good = Manifest::new(
            "r1",
            &QUICKNET,
            vec!["a".into(), "b".into()],
            vec![item("u1")],
        );
        let invalid = |doc: Manifest| matches!(doc.validate(), Err(Error::InvalidManifest(_)));

        assert!(invalid(Manifest {
            algo: "other".into(),
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrant_ids: vec![],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            items: vec![],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrant_ids: vec!["b".into(), "a".into()],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrant_ids: vec!["a".into(), "a".into()],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrant_ids: vec!["A".into()],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            items: vec![item("u1"), item("u1")],
            ..good
        }));
    }

    #[test]
    fn canonical_json_is_stable() {
        let doc = Manifest::new(
            "r1",
            &QUICKNET,
            vec!["01".into()],
            vec![ItemRef {
                id: "u1".into(),
                name: "Ticket".into(),
            }],
        );
        let json = doc.canonical_json();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.canonical_json(), json);
        assert!(json.starts_with(r#"{"algo":"rafi-draw-v1","raffle_id":"r1""#));
    }
}
