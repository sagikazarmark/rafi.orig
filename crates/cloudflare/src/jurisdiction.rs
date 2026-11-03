//! Which namespace the raffles' objects come from: plain Rust, so that the
//! choice is tested natively (`tests/jurisdiction.rs` includes this file)
//! while the rest of the crate is Workers glue.

/// The namespace the objects' IDs are created in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Namespace<'a> {
    /// The Durable Object namespace as bound: objects wherever Cloudflare
    /// places them. The default.
    Plain,
    /// The namespace restricted to a jurisdiction (`eu`, `fedramp`):
    /// `ns.jurisdiction(<it>)`.
    Jurisdiction(&'a str),
}

/// The namespace for the value of `RAFI_DO_JURISDICTION`: the plain one when
/// it is unset or empty (blank counts as empty), the jurisdiction it names
/// otherwise.
pub(crate) fn choose(value: Option<&str>) -> Namespace<'_> {
    match value.map(str::trim) {
        None | Some("") => Namespace::Plain,
        Some(jurisdiction) => Namespace::Jurisdiction(jurisdiction),
    }
}
