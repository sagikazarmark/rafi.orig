//! The manifest: everything that decides a drawing's outcome except the
//! randomness. Published (with its hash) when the drawing opens.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Hash, beacon::BeaconSource, draw::ALGO, error::Error, sha256};

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
    /// The draw algorithm and its version.
    pub algo: String,
    /// The drawing this manifest opens (its ID). Binds the manifest to one
    /// drawing; the manifest's own identity is its hash.
    pub drawing: String,
    /// Where the draws' randomness comes from.
    pub beacon: BeaconSource,
    /// Sorted, deduplicated, lowercase hex entrant IDs.
    pub entrants: Vec<String>,
    pub items: Vec<ItemRef>,
}

impl Manifest {
    /// `beacon` is where the draws' randomness will come from.
    pub fn new(
        drawing: impl Into<String>,
        beacon: BeaconSource,
        mut entrants: Vec<String>,
        items: Vec<ItemRef>,
    ) -> Self {
        for id in &mut entrants {
            *id = id.to_ascii_lowercase();
        }
        entrants.sort();
        entrants.dedup();
        Self {
            algo: ALGO.to_owned(),
            drawing: drawing.into(),
            beacon,
            entrants,
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
        if self.entrants.is_empty() {
            return invalid("nobody has entered");
        }
        if self.items.is_empty() {
            return invalid("there are no items");
        }
        if self
            .entrants
            .iter()
            .any(|id| *id != id.to_ascii_lowercase())
            || self.entrants.windows(2).any(|w| w[0] >= w[1])
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
    use crate::drand::chains::QUICKNET_HASH;

    const QUICKNET: BeaconSource = BeaconSource::Drand(QUICKNET_HASH);

    #[test]
    fn entrants_are_sorted_and_deduped() {
        let doc = Manifest::new(
            "r1",
            QUICKNET,
            vec!["bb".into(), "AA".into(), "aa".into()],
            vec![],
        );
        assert_eq!(doc.entrants, vec!["aa", "bb"]);
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
            QUICKNET,
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
            QUICKNET,
            vec!["a".into(), "b".into()],
            vec![item("u1")],
        );
        let invalid = |doc: Manifest| matches!(doc.validate(), Err(Error::InvalidManifest(_)));

        assert!(invalid(Manifest {
            algo: "other".into(),
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrants: vec![],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            items: vec![],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrants: vec!["b".into(), "a".into()],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrants: vec!["a".into(), "a".into()],
            ..good.clone()
        }));
        assert!(invalid(Manifest {
            entrants: vec!["A".into()],
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
            QUICKNET,
            vec!["01".into()],
            vec![ItemRef {
                id: "u1".into(),
                name: "Ticket".into(),
            }],
        );
        let json = doc.canonical_json();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.canonical_json(), json);
        assert_eq!(
            json,
            format!(
                r#"{{"algo":"rafi-draw-v1","drawing":"r1","beacon":{{"drand":"{QUICKNET_HASH}"}},"entrants":["01"],"items":[{{"id":"u1","name":"Ticket"}}]}}"#
            )
        );
    }
}
