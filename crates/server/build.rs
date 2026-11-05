//! Compiles in which Rafi this is (`rafi_server::build_info`): the version
//! and where its source is.
//!
//! - `RAFI_VERSION`, when the build sets it (a release does:
//!   `just cloudflare-artifact`), is the version, and its tag `v<version>` the source's
//!   ref;
//! - otherwise a checkout at a tag `v<version>` is that version;
//! - otherwise the crate's version, marked `-dev`, with the commit when there
//!   is one (`0.1.0-dev+3f9a1c2e4b5d`), whose ref is the commit.
//!
//! `RAFI_SOURCE_URL` names the source when the build sets it: a fork that
//! changes Rafi points its pages at its own (AGPL §13); otherwise the
//! workspace's `repository`.

use std::{path::Path, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=RAFI_VERSION");
    println!("cargo:rerun-if-env-changed=RAFI_SOURCE_URL");
    // The workspace's `repository`, which Cargo passes in, and this script.
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../../Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    let package = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();

    let (version, source_ref) = match std::env::var("RAFI_VERSION")
        .ok()
        .map(|v| v.trim().trim_start_matches('v').to_owned())
        .filter(|v| !v.is_empty())
    {
        Some(v) => (v.clone(), format!("v{v}")),
        None => from_git(&package),
    };
    let source = std::env::var("RAFI_SOURCE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("CARGO_PKG_REPOSITORY").ok())
        .unwrap_or_default();

    println!("cargo:rustc-env=RAFI_BUILD_VERSION={version}");
    println!(
        "cargo:rustc-env=RAFI_BUILD_SOURCE_URL={}",
        source.trim_end_matches('/')
    );
    println!("cargo:rustc-env=RAFI_BUILD_SOURCE_REF={source_ref}");
}

/// The version and the source's ref from the checkout, if this is one.
fn from_git(package: &str) -> (String, String) {
    watch_head();
    if let Some(tag) = git(&["describe", "--tags", "--exact-match", "--match", "v*"]) {
        return (tag.trim_start_matches('v').to_owned(), tag);
    }
    match git(&["rev-parse", "--short=12", "HEAD"]) {
        Some(commit) => (format!("{package}-dev+{commit}"), commit),
        None => (format!("{package}-dev"), "main".to_owned()),
    }
}

/// Builds again when the checkout moves to another commit or tag (not on
/// every change to the index, which would rebuild the server all the time).
fn watch_head() {
    let Some(dir) = git(&["rev-parse", "--git-dir"]) else {
        return;
    };
    let common = git(&["rev-parse", "--git-common-dir"]).unwrap_or_else(|| dir.clone());
    let mut watched = vec![
        Path::new(&dir).join("HEAD"),
        Path::new(&common).join("packed-refs"),
        Path::new(&common).join("refs/tags"),
    ];
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watched.push(Path::new(&common).join(branch));
    }
    for path in watched {
        // A path that doesn't exist would make every build run this again.
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!s.is_empty()).then_some(s)
}
