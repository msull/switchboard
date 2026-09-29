//! The data directory: where records live and how they are written.
//! Every write is a temp file, fsync, `.bak` of the previous version, and
//! a rename, under one writer lock shared by the runner and the commands.
//! The lock covers a whole read-modify-write, so the runner's pass and a
//! command from the terminal never interleave on a record.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// The writer lock, held while this lives.
#[derive(Debug)]
pub struct Lock(File);

impl Drop for Lock {
    fn drop(&mut self) {
        // Closing the file would release it too; saying so keeps the
        // release at the drop, not at whatever closes last.
        let _ = self.0.unlock();
    }
}

/// Where Dispatch keeps everything. `DISPATCH_DATA_DIR` overrides the
/// default under Application Support.
#[derive(Debug, Clone)]
pub struct DataDir {
    pub root: PathBuf,
}

impl DataDir {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `DISPATCH_DATA_DIR`, else `~/Library/Application Support/Dispatch`.
    pub fn from_env() -> Result<Self> {
        if let Some(dir) = std::env::var_os("DISPATCH_DATA_DIR") {
            return Ok(Self::new(dir));
        }
        let dirs = directories::ProjectDirs::from("com", "sadburger", "Dispatch")
            .context("no home directory")?;
        Ok(Self::new(dirs.data_dir()))
    }

    #[must_use]
    pub fn pipeline(&self, project: &str) -> PathBuf {
        self.root.join("pipelines").join(format!("{project}.toml"))
    }

    #[must_use]
    pub fn ticket_file(&self, id: &str) -> PathBuf {
        self.root.join("tickets").join(format!("{id}.json"))
    }

    /// The ticket's own directory: its pipeline copy and every attempt's
    /// artifacts.
    #[must_use]
    pub fn ticket_dir(&self, id: &str) -> PathBuf {
        self.root.join("tickets").join(id)
    }

    #[must_use]
    pub fn project_file(&self, project: &str) -> PathBuf {
        self.root.join("projects").join(format!("{project}.json"))
    }

    /// Every ticket file, in no particular order.
    pub fn ticket_files(&self) -> Result<Vec<PathBuf>> {
        let dir = self.root.join("tickets");
        let mut files = Vec::new();
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let path = entry?.path();
                    if path.extension().is_some_and(|e| e == "json") {
                        files.push(path);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context(format!("read {}", dir.display())),
        }
        files.sort();
        Ok(files)
    }

    fn lock_file(&self, name: &str) -> Result<File> {
        fs::create_dir_all(&self.root)?;
        let path = self.root.join(name);
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))
    }

    /// Take the writer lock, waiting for whoever holds it. Every
    /// read-modify-write of a record happens under it, from the runner
    /// and the commands alike, so two of them never interleave.
    pub fn lock(&self) -> Result<Lock> {
        let file = self.lock_file("lock")?;
        file.lock().context("take the writer lock")?;
        Ok(Lock(file))
    }

    /// Run `f` holding the writer lock.
    pub fn with_lock<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let lock = self.lock()?;
        let result = f();
        drop(lock);
        result
    }

    /// Become the one runner for this directory, or fail at once if
    /// another `dispatch run` holds it. Held for the process's life.
    pub fn claim_runner(&self) -> Result<Lock> {
        let file = self.lock_file("runner.lock")?;
        match file.try_lock() {
            Ok(()) => Ok(Lock(file)),
            Err(std::fs::TryLockError::WouldBlock) => bail!(
                "another dispatch run holds {}",
                self.root.join("runner.lock").display()
            ),
            Err(std::fs::TryLockError::Error(e)) => Err(e).context("take the runner lock"),
        }
    }
}

/// Write `bytes` to `path` so a crash leaves either the old file or the
/// new one, never a torn one and never none: temp file, fsync, the
/// previous version linked as `.bak` (the primary stays in place until
/// the rename replaces it), rename, directory fsync.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("a path with a parent")?;
    fs::create_dir_all(dir)?;
    let tmp = with_suffix(path, "tmp");
    {
        let mut file = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        let bak = with_suffix(path, "bak");
        match fs::remove_file(&bak) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {}", bak.display())),
        }
        fs::hard_link(path, &bak).with_context(|| format!("keep {}", bak.display()))?;
    }
    fs::rename(&tmp, path)?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Whether a record exists in either its primary or its backup.
#[must_use]
pub fn record_exists(path: &Path) -> bool {
    path.exists() || with_suffix(path, "bak").exists()
}

/// Read a JSON record, falling back to its `.bak` when the file is
/// unreadable.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
        }
        Err(e) => {
            let bak = with_suffix(path, "bak");
            match fs::read(&bak) {
                Ok(bytes) => serde_json::from_slice(&bytes)
                    .with_context(|| format!("parse {}", bak.display())),
                Err(_) => Err(e).with_context(|| format!("read {}", path.display())),
            }
        }
    }
}

pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    atomic_write(path, &bytes)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_write_keeps_the_previous_version_and_a_read_falls_back_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        write_json(&path, &vec![1]).unwrap();
        write_json(&path, &vec![1, 2]).unwrap();
        assert_eq!(read_json::<Vec<u32>>(&path).unwrap(), vec![1, 2]);
        assert!(path.with_file_name("t.json.bak").exists());
        assert!(!path.with_file_name("t.json.tmp").exists());
        fs::remove_file(&path).unwrap();
        assert_eq!(read_json::<Vec<u32>>(&path).unwrap(), vec![1]);
    }

    #[test]
    fn the_primary_is_never_absent_during_a_write() {
        // A hard link keeps the old bytes reachable as `.bak` while the
        // primary stays where it is; the rename swaps the new bytes in.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        write_json(&path, &vec![1]).unwrap();
        write_json(&path, &vec![2]).unwrap();
        write_json(&path, &vec![3]).unwrap();
        assert_eq!(read_json::<Vec<u32>>(&path).unwrap(), vec![3]);
        assert_eq!(
            read_json::<Vec<u32>>(&path.with_file_name("t.json.bak")).unwrap(),
            vec![2]
        );
        assert!(record_exists(&path));
        fs::remove_file(&path).unwrap();
        assert!(record_exists(&path), "the backup still counts");
    }

    #[test]
    fn only_one_runner_holds_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let first = data.claim_runner().unwrap();
        let second = data.claim_runner();
        assert!(second.is_err(), "{second:?}");
        assert!(
            second
                .unwrap_err()
                .to_string()
                .contains("another dispatch run"),
        );
        drop(first);
        data.claim_runner().unwrap();
    }

    #[test]
    fn the_lock_serialises_writers_and_lists_tickets() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        assert!(data.ticket_files().unwrap().is_empty());
        data.with_lock(|| write_json(&data.ticket_file("a1"), &1))
            .unwrap();
        data.with_lock(|| write_json(&data.ticket_file("b2"), &2))
            .unwrap();
        let names: Vec<String> = data
            .ticket_files()
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a1.json", "b2.json"]);
    }
}
