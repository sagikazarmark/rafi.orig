//! Verifies a quicknet beacon offline, with no network access.
//!
//! ```sh
//! cargo run --example verify
//! ```

use drand_beacon::{Beacon, Verifier, chains};

fn main() -> Result<(), drand_beacon::Error> {
    // As served by https://api.drand.sh/v2/chains/52db…e971/rounds/1000
    let body = br#"{"round":1000,"signature":"b44679b9a59af2ec876b1a6b1ad52ea9b1615fc3982b19576350f93447cb1125e342b73a8dd2bacbe47e4b6b63ed5e39"}"#;
    let beacon = Beacon::from_json(body)?;

    let verifier = Verifier::new(&chains::quicknet())?;
    let verified = verifier.verify_round(1000, &beacon)?;
    println!(
        "round {} verified, randomness {}",
        verified.round(),
        hex::encode(verified.randomness())
    );
    Ok(())
}
