//! Which Rafi this is, as the build compiled it in (`build.rs`): what
//! `/healthz`, the web app's footers and `whoami` show, and where the source
//! of exactly this version is (AGPL §13: users of a network service are
//! offered its source).

/// The version: a release's (`0.1.0`), or a development build's
/// (`0.1.0-dev+3f9a1c2e4b5d`).
pub const VERSION: &str = env!("RAFI_BUILD_VERSION");

/// The repository (`RAFI_SOURCE_URL` at build time, otherwise the
/// workspace's `repository`), without a trailing slash.
pub const SOURCE_URL: &str = env!("RAFI_BUILD_SOURCE_URL");

/// The source's ref for [`VERSION`]: a release's tag, or a commit.
pub const SOURCE_REF: &str = env!("RAFI_BUILD_SOURCE_REF");

/// The source of this version: the repository at [`SOURCE_REF`].
pub fn source_url() -> String {
    format!("{SOURCE_URL}/tree/{SOURCE_REF}")
}

/// The organizers' guide to an evening, for this version.
pub fn guide_url() -> String {
    format!("{SOURCE_URL}/blob/{SOURCE_REF}/docs/guide/running-a-raffle.md")
}

/// The independent verifier (`tools/verify.py`) of this version.
pub fn verifier_url() -> String {
    format!("{SOURCE_URL}/blob/{SOURCE_REF}/tools/verify.py")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_which_rafi() {
        assert!(
            !VERSION.is_empty() && !VERSION.starts_with('v'),
            "{VERSION}"
        );
        assert!(
            SOURCE_URL.starts_with("https://") && !SOURCE_URL.ends_with('/'),
            "{SOURCE_URL}"
        );
        assert!(!SOURCE_REF.is_empty());
        assert!(verifier_url().ends_with("/tools/verify.py"));
    }
}
