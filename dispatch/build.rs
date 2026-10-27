//! Stamps the app's version into `dispatch` as `SWITCHBOARD_VERSION`.
//! This crate's own version never moves, so it says nothing about which
//! release a runner was built from; the app's `Cargo.toml` does, and the
//! bundle ships both binaries built from it.

use std::fs;

fn main() {
    let manifest = "../Cargo.toml";
    println!("cargo:rerun-if-changed={manifest}");
    let text = fs::read_to_string(manifest).expect("the app's Cargo.toml beside this crate");
    // The first `version = "..."` line, the one `scripts/bump.sh` reads.
    let version = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))
        })
        .expect("a version = \"...\" line in the app's Cargo.toml");
    println!("cargo:rustc-env=SWITCHBOARD_VERSION={version}");
}
