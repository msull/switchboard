//! `dispatch check` from the command line: its exit codes, its report on
//! stdout, and a data directory left as it was.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ORCHARD: &str = include_str!("../../pipelines/examples/Orchard.toml");

fn cli(data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dispatch"))
        .args(args)
        .env("DISPATCH_DATA_DIR", data)
        .env("SWITCHBOARD_DATA_DIR", data.join("sb"))
        .env_remove("SWITCHBOARD_RECORD_ID")
        .output()
        .unwrap()
}

/// Every path under `dir`, with each file's contents.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.push((path.clone(), Vec::new()));
            out.extend(tree(&path));
        } else {
            out.push((path.clone(), std::fs::read(&path).unwrap()));
        }
    }
    out.sort();
    out
}

#[test]
fn a_broken_file_lists_every_problem_and_exits_one_changing_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let pipelines = dir.path().join("pipelines");
    std::fs::create_dir_all(&pipelines).unwrap();
    let broken = ORCHARD
        .replace("rebaser = \"rebaser\"", "rebaser = \"nobody\"")
        .replace(
            "[[stages]]\nname = \"plan\"",
            "[[stages]]\nname = \"plan\"\ngates = 1",
        );
    assert_ne!(broken, ORCHARD);
    std::fs::write(pipelines.join("P.toml"), &broken).unwrap();
    let before = tree(dir.path());
    let out = cli(dir.path(), &["check", "P"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("error: [policy] rebaser names an unknown operator \"nobody\""),
        "{stdout}"
    );
    // Stored as P, the file names another project.
    assert!(
        stdout.contains("error: the pipeline file names project \"Orchard\", not \"P\""),
        "{stdout}"
    );
    assert!(stdout.contains("2 errors, 1 warning"), "{stdout}");
    assert!(stdout.contains("warning: stages["), "{stdout}");
    assert!(
        stdout.contains(".gates is not a key the parser keeps"),
        "{stdout}"
    );
    assert_eq!(tree(dir.path()), before);
}

#[test]
fn a_clean_example_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pipelines/examples/Orchard.toml"
    );
    let out = cli(dir.path(), &["check", path]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert_eq!(stdout.trim(), format!("{path}: ok"));
}

#[test]
fn check_with_no_argument_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(cli(dir.path(), &["check"]).status.code(), Some(64));
}
