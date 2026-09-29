//! The control port's adapters: the operations log on disk, and (next)
//! the socket that carries requests from another process.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::ports::control::{OpLine, Operations};

/// The log's file name inside the data directory.
pub const OPERATIONS_FILE: &str = "operations.log";

/// `<data dir>/operations.log`: one JSON line per entry, appended with
/// `O_APPEND` and synced before `append` returns, owner-readable only.
/// Existing lines are read once at open and kept in memory for `find`.
pub struct OperationsLog {
    path: PathBuf,
    lines: Vec<OpLine>,
}

impl OperationsLog {
    /// Open (creating if absent) the log in `data_dir`.
    pub fn open(data_dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(OPERATIONS_FILE);
        let mut lines = Vec::new();
        match File::open(&path) {
            Ok(file) => {
                for line in BufReader::new(file).lines() {
                    let line = line?;
                    match serde_json::from_str::<OpLine>(&line) {
                        Ok(entry) => lines.push(entry),
                        // A torn last line from a crash mid-write is the
                        // one expected malformation; anything else is
                        // logged and skipped the same way.
                        Err(e) => log::warn!("{}: skipping a line: {e}", path.display()),
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self { path, lines })
    }

    /// Every line, oldest first.
    #[must_use]
    pub fn lines(&self) -> &[OpLine] {
        &self.lines
    }
}

impl Operations for OperationsLog {
    fn append(&mut self, line: &OpLine) -> std::io::Result<()> {
        let mut text = serde_json::to_string(line)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        text.push('\n');
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        self.lines.push(line.clone());
        Ok(())
    }

    fn find(&self, op: &str) -> Vec<OpLine> {
        self.lines
            .iter()
            .filter(|l| l.op() == op)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;

    fn requested(op: &str) -> OpLine {
        OpLine::Requested {
            op: op.into(),
            kind: "session.new".into(),
            ids: vec!["s1".into()],
            at: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn lines_survive_a_reopen_and_a_torn_tail_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = OperationsLog::open(dir.path()).unwrap();
        log.append(&requested("a")).unwrap();
        log.append(&OpLine::Replied {
            op: "a".into(),
            reply: "{\"reply\":\"launched\"}".into(),
            at: SystemTime::UNIX_EPOCH,
        })
        .unwrap();
        log.append(&requested("b")).unwrap();
        // A crash mid-write leaves a partial line.
        let path = dir.path().join(OPERATIONS_FILE);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"{\"line\":\"requested\",\"op\":\"c")
            .unwrap();
        drop(file);

        let reopened = OperationsLog::open(dir.path()).unwrap();
        assert_eq!(reopened.lines().len(), 3);
        assert_eq!(reopened.find("a").len(), 2);
        assert_eq!(reopened.find("b"), vec![requested("b")]);
        assert!(reopened.find("c").is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
