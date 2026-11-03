//! The Durable Object namespace's choice (`src/jurisdiction.rs`): the plain
//! namespace unless `RAFI_DO_JURISDICTION` names a jurisdiction. The crate
//! itself is empty off wasm32, so the module is included here directly.

#[path = "../src/jurisdiction.rs"]
mod jurisdiction;

use jurisdiction::{Namespace, choose};

#[test]
fn unset_is_the_plain_namespace() {
    assert_eq!(choose(None), Namespace::Plain);
}

#[test]
fn empty_or_blank_is_the_plain_namespace() {
    assert_eq!(choose(Some("")), Namespace::Plain);
    assert_eq!(choose(Some("  ")), Namespace::Plain);
}

#[test]
fn a_value_is_that_jurisdiction() {
    assert_eq!(choose(Some("eu")), Namespace::Jurisdiction("eu"));
    assert_eq!(
        choose(Some(" fedramp ")),
        Namespace::Jurisdiction("fedramp")
    );
}
