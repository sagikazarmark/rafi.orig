//! The manifest: everything that decides a raffle's outcome except the
//! randomness. Published (with its hash) when the raffle opens.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    algorithm::ALGO,
    beacon::BeaconSource,
    error::Error,
    hash::ManifestHash,
    id::{EntrantId, ItemId, RaffleId},
};

/// An item as the manifest lists it: one thing one entrant can win.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestItem {
    pub id: ItemId,
    pub name: String,
}

/// Who is in, what can be won, and how the winners will be drawn.
///
/// Field order is fixed and the manifest contains no maps or floats, so
/// `serde_json` serialization is canonical. The exact serialized string is
/// published alongside the hash, so verifiers never need to re-serialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The draw algorithm and its version.
    pub algo: String,
    /// The raffle this manifest opens (its ID). Binds the manifest to one
    /// raffle; the manifest's own identity is its hash.
    pub raffle: RaffleId,
    /// Where the draws' randomness comes from.
    pub beacon: BeaconSource,
    /// Sorted, deduplicated, lowercase entrant IDs (rafi's are hex).
    pub entrants: Vec<EntrantId>,
    pub items: Vec<ManifestItem>,
}

impl Manifest {
    /// `beacon` is where the draws' randomness will come from.
    pub fn new(
        raffle: impl Into<RaffleId>,
        beacon: BeaconSource,
        entrants: Vec<EntrantId>,
        items: Vec<ManifestItem>,
    ) -> Self {
        let mut entrants: Vec<EntrantId> = entrants
            .into_iter()
            .map(|id| id.as_str().to_ascii_lowercase().into())
            .collect();
        entrants.sort();
        entrants.dedup();
        Self {
            algo: ALGO.to_owned(),
            raffle: raffle.into(),
            beacon,
            entrants,
            items,
        }
    }

    /// The canonical serialization that gets hashed and published.
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(self).expect("manifest serializes")
    }

    /// The hash of this manifest's canonical JSON.
    pub fn hash(&self) -> ManifestHash {
        ManifestHash::of(&self.canonical_json())
    }

    /// The idempotency key for opening a raffle from this manifest: the same
    /// manifest opens once.
    pub fn open_key(&self) -> String {
        format!("open-{}", self.hash())
    }

    /// Checks that the manifest is one [`Manifest::new`] could have built for
    /// a raffle that can be drawn: the current algorithm, at least one entrant and one
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
            .any(|id| id.as_str() != id.as_str().to_ascii_lowercase())
            || self.entrants.windows(2).any(|w| w[0] >= w[1])
        {
            return invalid("entrant IDs are not sorted, unique and lowercase");
        }
        let mut seen = HashSet::new();
        if !self.items.iter().all(|item| seen.insert(&item.id)) {
            return invalid("duplicate item IDs");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drand::chains::QUICKNET_HASH;

    const QUICKNET: BeaconSource = BeaconSource::QUICKNET;

    #[test]
    fn entrants_are_sorted_and_deduped() {
        let manifest = Manifest::new(
            "d1",
            QUICKNET,
            vec!["bb".into(), "AA".into(), "aa".into()],
            vec![],
        );
        assert_eq!(manifest.entrants, vec!["aa", "bb"]);
    }

    fn item(id: &str) -> ManifestItem {
        ManifestItem {
            id: id.into(),
            name: "Item".into(),
        }
    }

    #[test]
    fn manifests_built_by_new_validate() {
        let manifest = Manifest::new(
            "d1",
            QUICKNET,
            vec!["B".into(), "a".into()],
            vec![item("item-1")],
        );
        assert_eq!(manifest.validate(), Ok(()));
        assert_eq!(manifest.open_key(), format!("open-{}", manifest.hash()));
    }

    #[test]
    fn non_canonical_or_empty_manifests_are_invalid() {
        let good = Manifest::new(
            "d1",
            QUICKNET,
            vec!["a".into(), "b".into()],
            vec![item("item-1")],
        );
        let invalid = |m: Manifest| matches!(m.validate(), Err(Error::InvalidManifest(_)));

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
            items: vec![item("item-1"), item("item-1")],
            ..good
        }));
    }

    #[test]
    fn canonical_json_is_stable() {
        let manifest = Manifest::new("d1", QUICKNET, vec!["01".into()], vec![item("item-1")]);
        let json = manifest.canonical_json();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.canonical_json(), json);
        assert_eq!(
            json,
            format!(
                r#"{{"algo":"rafi-draw-v1","raffle":"d1","beacon":{{"drand":"{QUICKNET_HASH}"}},"entrants":["01"],"items":[{{"id":"item-1","name":"Item"}}]}}"#
            )
        );
    }

    /// The canonical form's escaping, as the draw algorithm pins it: only
    /// `"`, `\` and the C0 controls are escaped (the five short forms, else
    /// `\u00xx` in lowercase); `/`, U+007F, U+2028/2029 and every non-ASCII
    /// character are written raw. A `serde_json` that changed this would
    /// make legitimate published manifests "not canonical".
    #[test]
    fn canonical_escaping_is_pinned() {
        let manifest = Manifest::new(
            "d1",
            QUICKNET,
            vec!["01".into()],
            vec![ManifestItem {
                id: "i".into(),
                name: "a\"b\\c/d\u{8}\t\n\u{c}\r\u{1}\u{1f}\u{7f}\u{2028}é🎉".into(),
            }],
        );
        let json = manifest.canonical_json();
        let name = json.split(r#""name":"#).nth(1).unwrap();
        assert_eq!(
            name,
            "\"a\\\"b\\\\c/d\\b\\t\\n\\f\\r\\u0001\\u001f\u{7f}\u{2028}é🎉\"}]}"
        );
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, manifest);
    }
}
