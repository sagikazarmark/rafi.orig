use alloc::{string::String, vec::Vec};

use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeStruct};

use crate::{ChainHash, Error, sha256};

/// An untrusted beacon, as served by a relay. Turn it into a
/// [`VerifiedBeacon`] with a [`crate::Verifier`].
///
/// Serde depends on the format:
/// - Human-readable formats (JSON) use the v1 relay shape: hex strings, with
///   `randomness`, and `previous_signature` only when present. Deserializing
///   accepts v1 and v2 relay bodies; a `randomness` field, if present, must
///   equal `sha256(signature)`, and an empty `previous_signature` counts as absent.
/// - Binary formats (postcard, bincode) use a fixed
///   `(round, signature, previous_signature)` layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Beacon {
    /// The round number.
    pub round: u64,
    /// The signature bytes as published.
    pub signature: Vec<u8>,
    /// The previous round's signature (chained schemes only).
    pub previous_signature: Option<Vec<u8>>,
}

impl Beacon {
    /// A beacon from hex strings (e.g. from a stored log).
    pub fn from_hex(
        round: u64,
        signature: &str,
        previous_signature: Option<&str>,
    ) -> Result<Self, Error> {
        let signature = hex::decode(signature).map_err(|_| Error::parse("signature"))?;
        let previous_signature = match previous_signature {
            None | Some("") => None,
            Some(p) => Some(hex::decode(p).map_err(|_| Error::parse("previous signature"))?),
        };
        Ok(Self {
            round,
            signature,
            previous_signature,
        })
    }

    /// Parses a v1 or v2 relay response body.
    pub fn from_json(body: &[u8]) -> Result<Self, Error> {
        let json: BeaconJson = serde_json::from_slice(body).map_err(|_| Error::parse("beacon"))?;
        json.into_beacon()
    }

    /// `sha256(signature)`: the beacon's randomness, once verified.
    pub fn randomness(&self) -> [u8; 32] {
        sha256(&[&self.signature])
    }
}

#[derive(Deserialize)]
struct BeaconJson {
    round: u64,
    signature: String,
    #[serde(default)]
    previous_signature: Option<String>,
    #[serde(default)]
    randomness: Option<String>,
}

impl BeaconJson {
    fn into_beacon(self) -> Result<Beacon, Error> {
        let beacon = Beacon::from_hex(
            self.round,
            &self.signature,
            self.previous_signature.as_deref(),
        )?;
        if let Some(r) = self.randomness {
            let mut claimed = [0u8; 32];
            hex::decode_to_slice(&r, &mut claimed).map_err(|_| Error::parse("randomness"))?;
            if claimed != beacon.randomness() {
                return Err(Error::RandomnessMismatch);
            }
        }
        Ok(beacon)
    }
}

#[derive(Serialize, Deserialize)]
struct BeaconBin {
    round: u64,
    signature: Vec<u8>,
    previous_signature: Option<Vec<u8>>,
}

fn serialize_json<S: Serializer>(
    s: S,
    name: &'static str,
    chain: Option<&ChainHash>,
    beacon: &Beacon,
    randomness: &[u8; 32],
) -> Result<S::Ok, S::Error> {
    let fields =
        3 + usize::from(chain.is_some()) + usize::from(beacon.previous_signature.is_some());
    let mut st = s.serialize_struct(name, fields)?;
    if let Some(chain) = chain {
        st.serialize_field("chain", chain)?;
    }
    st.serialize_field("round", &beacon.round)?;
    st.serialize_field("signature", &hex::encode(&beacon.signature))?;
    if let Some(prev) = &beacon.previous_signature {
        st.serialize_field("previous_signature", &hex::encode(prev))?;
    } else {
        st.skip_field("previous_signature")?;
    }
    st.serialize_field("randomness", &hex::encode(randomness))?;
    st.end()
}

impl Serialize for Beacon {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            serialize_json(s, "Beacon", None, self, &self.randomness())
        } else {
            BeaconBin {
                round: self.round,
                signature: self.signature.clone(),
                previous_signature: self.previous_signature.clone(),
            }
            .serialize(s)
        }
    }
}

impl<'de> Deserialize<'de> for Beacon {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if d.is_human_readable() {
            BeaconJson::deserialize(d)?
                .into_beacon()
                .map_err(serde::de::Error::custom)
        } else {
            let b = BeaconBin::deserialize(d)?;
            Ok(Self {
                round: b.round,
                signature: b.signature,
                previous_signature: b.previous_signature,
            })
        }
    }
}

/// A beacon that passed verification against a chain.
///
/// Only a [`crate::Verifier`] creates one; it can be serialized but not
/// deserialized. Persist the [`Beacon`] instead and verify it again on load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedBeacon {
    pub(crate) chain: ChainHash,
    pub(crate) beacon: Beacon,
    pub(crate) randomness: [u8; 32],
}

impl VerifiedBeacon {
    /// The round number.
    pub fn round(&self) -> u64 {
        self.beacon.round
    }

    /// The verified randomness, `sha256(signature)`.
    pub fn randomness(&self) -> &[u8; 32] {
        &self.randomness
    }

    /// Hash of the chain this beacon was verified against. Callers accepting
    /// several chains must compare it with the one they expect.
    pub fn chain(&self) -> &ChainHash {
        &self.chain
    }

    /// The verified beacon. For unchained schemes `previous_signature` is `None`.
    pub fn beacon(&self) -> &Beacon {
        &self.beacon
    }

    /// The plain beacon, e.g. to persist it.
    pub fn into_beacon(self) -> Beacon {
        self.beacon
    }
}

impl Serialize for VerifiedBeacon {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() {
            serialize_json(
                s,
                "VerifiedBeacon",
                Some(&self.chain),
                &self.beacon,
                &self.randomness,
            )
        } else {
            (&self.chain, &self.beacon).serialize(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIG: &str = "b44679b9a59af2ec876b1a6b1ad52ea9b1615fc3982b19576350f93447cb1125e342b73a8dd2bacbe47e4b6b63ed5e39";
    const RANDOMNESS: &str = "fe290beca10872ef2fb164d2aa4442de4566183ec51c56ff3cd603d930e54fdd";

    #[test]
    fn parses_v1_v2_and_old_shapes() {
        let v1 =
            alloc::format!(r#"{{"round":1000,"signature":"{SIG}","randomness":"{RANDOMNESS}"}}"#);
        let v2 = alloc::format!(r#"{{"round":1000,"signature":"{SIG}"}}"#);
        let a = Beacon::from_json(v1.as_bytes()).unwrap();
        let b: Beacon = serde_json::from_str(&v2).unwrap();
        assert_eq!(a, b);
        assert_eq!(hex::encode(a.randomness()), RANDOMNESS);
        assert_eq!(a.previous_signature, None);
    }

    #[test]
    fn rejects_wrong_randomness_and_bad_hex() {
        let bad = alloc::format!(
            r#"{{"round":1,"signature":"{SIG}","randomness":"{}"}}"#,
            "00".repeat(32)
        );
        assert_eq!(
            Beacon::from_json(bad.as_bytes()),
            Err(Error::RandomnessMismatch)
        );
        assert!(Beacon::from_json(br#"{"round":1,"signature":"zz"}"#).is_err());
    }

    #[test]
    fn empty_previous_signature_is_none() {
        let b = Beacon::from_json(
            alloc::format!(r#"{{"round":1,"signature":"{SIG}","previous_signature":""}}"#)
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(b.previous_signature, None);
    }

    #[test]
    fn json_output_is_v1_shaped() {
        let b = Beacon::from_hex(1000, SIG, None).unwrap();
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(
            json,
            alloc::format!(r#"{{"round":1000,"signature":"{SIG}","randomness":"{RANDOMNESS}"}}"#)
        );
        assert_eq!(serde_json::from_str::<Beacon>(&json).unwrap(), b);
    }

    #[test]
    fn binary_round_trip() {
        for prev in [None, Some("aa")] {
            let b = Beacon::from_hex(7, SIG, prev).unwrap();
            let bytes = postcard::to_allocvec(&b).unwrap();
            assert_eq!(postcard::from_bytes::<Beacon>(&bytes).unwrap(), b);
        }
    }
}
