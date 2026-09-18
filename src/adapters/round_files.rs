//! Round files on the local disk.

use std::path::{Path, PathBuf};

use crate::ports::round_files::{FileStamp, Probed, RoundFiles};

/// The width of a first line worth keeping: the verdict compare needs
/// one sentence, never the whole feedback.
const FIRST_LINE_CAP: usize = 512;

#[derive(Debug, Default)]
pub struct DiskRoundFiles;

impl RoundFiles for DiskRoundFiles {
    fn probe(&self, path: &Path) -> Option<Probed> {
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() {
            return None;
        }
        let modified = meta.modified().ok()?;
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let first_line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .chars()
            .take(FIRST_LINE_CAP)
            .collect();
        Some(Probed {
            stamp: FileStamp {
                modified,
                len: meta.len(),
            },
            first_line,
        })
    }

    fn snapshot(&self, files: &[PathBuf], dir: &Path, note: Option<&str>) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for file in files {
            let Some(name) = file.file_name() else {
                continue;
            };
            if !file.is_file() {
                continue;
            }
            let dst = dir.join(name);
            std::fs::copy(file, &dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        }
        if let Some(note) = note {
            let dst = dir.join("user-feedback.md");
            std::fs::write(&dst, note).map_err(|e| format!("{}: {e}", dst.display()))?;
        }
        Ok(())
    }

    fn remove(&self, files: &[PathBuf]) -> Result<(), String> {
        for file in files {
            match std::fs::remove_file(file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", file.display())),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reads_the_first_non_blank_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("feedback-1.md");
        assert_eq!(DiskRoundFiles.probe(&path), None);
        std::fs::write(&path, "\n  No further feedback.  \nmore\n").unwrap();
        let probed = DiskRoundFiles.probe(&path).unwrap();
        assert_eq!(probed.first_line, "No further feedback.");
        assert_eq!(probed.stamp.len, 31);
    }

    #[test]
    fn snapshot_copies_what_exists_and_remove_ignores_the_missing() {
        let dir = tempfile::tempdir().unwrap();
        let plan = dir.path().join("plan.md");
        let missing = dir.path().join("response-1.md");
        std::fs::write(&plan, "# plan").unwrap();
        let snap = dir.path().join("snap").join("round-1");
        DiskRoundFiles
            .snapshot(&[plan.clone(), missing.clone()], &snap, Some("mine"))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(snap.join("plan.md")).unwrap(),
            "# plan"
        );
        assert_eq!(
            std::fs::read_to_string(snap.join("user-feedback.md")).unwrap(),
            "mine"
        );
        assert!(!snap.join("response-1.md").exists());
        DiskRoundFiles.remove(&[plan.clone(), missing]).unwrap();
        assert!(!plan.exists());
    }
}
