//! Where the Worker's legal documents come from. They are bundled into the
//! Worker at build time (`include_str!`), from `legal/` in this directory by
//! default, or from the directory `RAFI_LEGAL_DIR` names, so an installation
//! keeps its reviewed `terms.md` and `privacy.md` outside the tree and an
//! upgrade is a plain pull. The paths are handed to the crate as
//! `RAFI_LEGAL_TERMS` and `RAFI_LEGAL_PRIVACY`, absolute; both files are
//! watched, so an edit outside the tree still rebuilds the Worker.

use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=RAFI_LEGAL_DIR");
    let dir = match env::var_os("RAFI_LEGAL_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo")).join("legal"),
    };
    let dir = dir
        .canonicalize()
        .unwrap_or_else(|e| panic!("RAFI_LEGAL_DIR {}: {e}", dir.display()));
    for (var, file) in [
        ("RAFI_LEGAL_TERMS", "terms.md"),
        ("RAFI_LEGAL_PRIVACY", "privacy.md"),
    ] {
        let path = dir.join(file);
        assert!(
            path.is_file(),
            "{} is missing: the Worker bundles the installation's legal documents from RAFI_LEGAL_DIR (default: apps/worker/legal)",
            path.display()
        );
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-env={var}={}", path.display());
    }
}
