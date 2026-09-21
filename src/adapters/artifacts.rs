//! Declared outputs on the local disk, through `glob`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::ports::artifacts::ArtifactFinder;

/// Files per run kept at most; a pattern like `**/*` would otherwise
/// pin the whole tree to the record.
const MAX_ARTIFACTS: usize = 50;

/// File times are whole seconds on some filesystems; a file written in
/// the same second the run started still counts.
const SLACK: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct DiskArtifacts;

impl ArtifactFinder for DiskArtifacts {
    fn find(&self, cwd: &Path, patterns: &[String], since: SystemTime) -> Vec<PathBuf> {
        let since = since.checked_sub(SLACK).unwrap_or(since);
        let mut out: Vec<PathBuf> = Vec::new();
        for pattern in patterns {
            let full = cwd.join(pattern);
            let Some(text) = full.to_str() else {
                continue;
            };
            let Ok(paths) = glob::glob(text) else {
                continue;
            };
            for path in paths.flatten() {
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                let fresh = meta.modified().is_ok_and(|m| m >= since);
                if meta.is_file() && fresh && !out.contains(&path) {
                    out.push(path);
                }
                if out.len() >= MAX_ARTIFACTS {
                    break;
                }
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_declared_files_modified_since_the_run_started() {
        let dir = tempfile::tempdir().unwrap();
        let reports = dir.path().join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        let old = reports.join("old.pdf");
        std::fs::write(&old, "x").unwrap();
        let long_ago = SystemTime::now() - Duration::from_secs(3600);
        filetime_set(&old, long_ago);
        let new = reports.join("new.pdf");
        std::fs::write(&new, "y").unwrap();
        std::fs::write(reports.join("notes.txt"), "z").unwrap();
        let since = SystemTime::now() - Duration::from_secs(60);
        let found = DiskArtifacts.find(
            dir.path(),
            &["reports/*.pdf".into(), "reports/*.pdf".into()],
            since,
        );
        assert_eq!(found, vec![new]);
        assert!(DiskArtifacts.find(dir.path(), &[], since).is_empty());
        assert!(
            DiskArtifacts
                .find(dir.path(), &["[".into()], since)
                .is_empty()
        );
    }

    fn filetime_set(path: &Path, t: SystemTime) {
        let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(t).unwrap();
    }
}
