//! SHA-256, and the hashes a raffle is bound to.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A SHA-256 digest.
pub type Hash = [u8; 32];

pub(crate) fn sha256(parts: &[&[u8]]) -> Hash {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize().into()
}

/// A drand beacon's randomness: `SHA256(signature)`.
pub(crate) fn randomness(signature: &[u8]) -> Hash {
    sha256(&[signature])
}

/// `SHA256(canonical manifest)`: published when the raffle opens, mixed into
/// every draw's seed. Written as lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ManifestHash(Hash);

impl ManifestHash {
    /// The hash of a manifest's canonical JSON.
    pub fn of(canonical_json: &str) -> Self {
        Self(sha256(&[canonical_json.as_bytes()]))
    }

    pub fn as_bytes(&self) -> &Hash {
        &self.0
    }
}

impl fmt::Display for ManifestHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for ManifestHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ManifestHash({self})")
    }
}

/// Not 32 bytes of hex.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a 32-byte hex hash")]
pub struct InvalidHash;

impl FromStr for ManifestHash {
    type Err = InvalidHash;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|_| InvalidHash)?;
        Ok(Self(bytes.try_into().map_err(|_| InvalidHash)?))
    }
}

impl Serialize for ManifestHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ManifestHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_hashes_are_lowercase_hex() {
        let hash = ManifestHash::of("{}");
        let hex = hash.to_string();
        assert_eq!(hex, hex.to_ascii_lowercase());
        assert_eq!(hex.parse::<ManifestHash>(), Ok(hash));
        assert_eq!(hex.to_ascii_uppercase().parse::<ManifestHash>(), Ok(hash));
        assert_eq!(serde_json::to_string(&hash).unwrap(), format!("\"{hex}\""));
        assert_eq!("abc".parse::<ManifestHash>(), Err(InvalidHash));
    }
}
