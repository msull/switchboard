//! `scripts/fold-changelog.sh`, the step of `scripts/bump.sh` that moves
//! the `changelog.d/` fragments into a version section of the
//! changelog. Each test runs the script with `sh` on a hand-written
//! changelog in a temp directory, so the real `CHANGELOG.md` is never
//! touched.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const HEAD: &str = "# Changelog\n\nThe rule.\n\n## [Unreleased]\n";
const OLDER: &str = "## [0.7.0] - 2026-10-25\n\n### Added\n\n- An older thing.\n";

/// Writes `changelog` and each `(path, text)` fragment under
/// `changelog.d/`, runs the fold for 0.7.1 dated 2026-11-01, and
/// returns its output.
fn fold(root: &Path, changelog: &str, fragments: &[(&str, &str)]) -> Output {
    fs::write(root.join("CHANGELOG.md"), changelog).unwrap();
    let dir = root.join("changelog.d");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("README.md"), "not a fragment\n").unwrap();
    for (path, text) in fragments {
        let file = dir.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/fold-changelog.sh");
    Command::new("sh")
        .arg(script)
        .args(["CHANGELOG.md", "changelog.d", "0.7.1", "2026-11-01"])
        .current_dir(root)
        .output()
        .unwrap()
}

fn read(root: &Path) -> String {
    fs::read_to_string(root.join("CHANGELOG.md")).unwrap()
}

fn assert_ok(out: &Output) {
    assert!(
        out.status.success(),
        "fold failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn no_fragments_matches_the_old_insertion() {
    // An empty fold: the new heading after Unreleased and every
    // following line untouched.
    let tmp = tempfile::tempdir().unwrap();
    let input = format!("{HEAD}\n{OLDER}");
    assert_ok(&fold(tmp.path(), &input, &[]));
    assert_eq!(
        read(tmp.path()),
        format!("{HEAD}\n## [0.7.1] - 2026-11-01\n\n{OLDER}")
    );
}

#[test]
fn fragments_group_in_fixed_order() {
    let tmp = tempfile::tempdir().unwrap();
    let input = format!("{HEAD}\n{OLDER}");
    let out = fold(
        tmp.path(),
        &input,
        &[
            ("dispatch/30-b.md", "Fixed: B fixed.\n"),
            (
                "dispatch/10-a.md",
                "Changed: A changed.\n\nAdded: A added.\n",
            ),
            ("dispatch/20-c.md", "Added: C added."),
        ],
    );
    assert_ok(&out);
    assert_eq!(
        read(tmp.path()),
        format!(
            "{HEAD}\n## [0.7.1] - 2026-11-01\n\n\
             ### Added\n\n- A added.\n- C added.\n\n\
             ### Changed\n\n- A changed.\n\n\
             ### Fixed\n\n- B fixed.\n\n{OLDER}"
        )
    );
}

#[test]
fn unreleased_entries_join_the_group_of_the_same_name() {
    let tmp = tempfile::tempdir().unwrap();
    let input = format!("{HEAD}\n### Added\n\n- Written by hand.\n\n{OLDER}");
    assert_ok(&fold(
        tmp.path(),
        &input,
        &[("x.md", "Fixed: From a file.\n"), ("y.md", "Added: New.\n")],
    ));
    assert_eq!(
        read(tmp.path()),
        format!(
            "{HEAD}\n## [0.7.1] - 2026-11-01\n\n### Added\n\n- New.\n- Written by hand.\n\n\
             ### Fixed\n\n- From a file.\n\n{OLDER}"
        )
    );
}

#[test]
fn unreleased_text_outside_a_group_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let input = format!("{HEAD}\n- Written by hand.\n\n{OLDER}");
    let out = fold(tmp.path(), &input, &[("x.md", "Fixed: From a file.\n")]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Written by hand"));
    assert_eq!(read(tmp.path()), input);
    assert!(tmp.path().join("changelog.d/x.md").exists());
}

#[test]
fn fragments_are_removed_and_readme_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let input = format!("{HEAD}\n{OLDER}");
    let out = fold(
        tmp.path(),
        &input,
        &[
            ("dispatch/1-a.md", "Added: A.\n"),
            ("top.md", "Fixed: T.\n"),
        ],
    );
    assert_ok(&out);
    let dir = tmp.path().join("changelog.d");
    let left: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["README.md"]);
}

#[test]
fn a_bad_line_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let input = format!("{HEAD}\n{OLDER}");
    let out = fold(
        tmp.path(),
        &input,
        &[
            ("a.md", "Added: Fine.\n"),
            ("dispatch/2-bad.md", "Fixed: Fine.\n- no group\n"),
        ],
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("dispatch/2-bad.md:2"), "{stderr}");
    assert_eq!(read(tmp.path()), input);
    let dir = tmp.path().join("changelog.d");
    assert!(dir.join("a.md").exists());
    assert!(dir.join("dispatch/2-bad.md").exists());
}
