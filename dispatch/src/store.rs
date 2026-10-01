//! The data directory: where records live and how they are written.
//! Every write is a temp file, fsync, `.bak` of the previous version, and
//! a rename, under one writer lock shared by the runner and the commands.
//! The lock covers a whole read-modify-write, so the runner's pass and a
//! command from the terminal never interleave on a record.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::ticket::{ProjectState, Ticket};

/// The format of a ticket or project record this build reads and
/// writes. A record carries it as `version`; one written before records
/// carried a version reads as 0 and is brought up by `migrate`. A
/// record above it was written by a newer `dispatch` and is refused
/// both ways, so this build never drops fields it does not know.
pub const RECORD_VERSION: u32 = 1;

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

/// Settings of a data directory, at `<data>/settings.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where tickets' trees go; `~` is expanded when read.
    pub worktrees: Option<PathBuf>,
}

/// `~/.dispatch/worktrees`, when there is a home.
#[must_use]
pub fn default_worktrees_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".dispatch").join("worktrees"))
}

/// A leading `~` as the home directory, as a shell would read it.
#[must_use]
pub fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    if text == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    path.to_path_buf()
}

/// Why a path is not safe to hand to a repository's tooling: any
/// whitespace or shell-special character in it, since a task that
/// interpolates its own location into a shell string splits there.
#[must_use]
pub fn shell_unsafe(path: &Path) -> Option<String> {
    let text = path.to_string_lossy();
    let bad: Vec<char> = text
        .chars()
        .filter(|c| c.is_whitespace() || "'\"$`\\;&|<>(){}[]*?!#".contains(*c))
        .collect();
    if bad.is_empty() {
        return None;
    }
    let shown: String = bad
        .iter()
        .map(|c| format!("{c:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "{} holds {shown}; a repository's own tooling may split a command there",
        path.display()
    ))
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
        let dirs =
            directories::ProjectDirs::from("", "", "Dispatch").context("no home directory")?;
        Ok(Self::new(dirs.data_dir()))
    }

    #[must_use]
    pub fn pipeline(&self, project: &str) -> PathBuf {
        self.root.join("pipelines").join(format!("{project}.toml"))
    }

    /// The project's pipeline for tickets taken from pull requests.
    #[must_use]
    pub fn pr_pipeline(&self, project: &str) -> PathBuf {
        self.root
            .join("pipelines")
            .join(format!("{project}.pr.toml"))
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

    /// Dispatch's own clone of a project's repository.
    #[must_use]
    pub fn repo_dir(&self, project: &str) -> PathBuf {
        self.root.join("repos").join(project)
    }

    /// Where tickets' trees go unless a pipeline says otherwise: the
    /// `worktrees` setting, else `~/.dispatch/worktrees`. Never under
    /// the data directory itself, whose path on macOS holds a space
    /// that a repository's own tooling may not survive.
    #[must_use]
    pub fn worktrees_dir(&self) -> PathBuf {
        if let Some(dir) = self.settings().worktrees {
            return dir;
        }
        default_worktrees_dir().unwrap_or_else(|| self.root.join("worktrees"))
    }

    #[must_use]
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// The directory's own settings; absent or unreadable reads as
    /// defaults.
    #[must_use]
    pub fn settings(&self) -> Settings {
        read_json(&self.settings_file()).unwrap_or_default()
    }

    pub fn write_settings(&self, settings: &Settings) -> Result<()> {
        write_json(&self.settings_file(), settings)
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

/// A ticket record, its version checked and migrated to this build's.
pub fn read_ticket(path: &Path) -> Result<Ticket> {
    read_record(path)
}

/// A project's state record, its version checked and migrated.
pub fn read_project(path: &Path) -> Result<ProjectState> {
    read_record(path)
}

fn read_record<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let value: Value = read_json(path)?;
    let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version > u64::from(RECORD_VERSION) {
        bail!(
            "{} is version {version}, written by a newer dispatch; update this one",
            path.display()
        );
    }
    serde_json::from_value(migrate(value)).with_context(|| format!("parse {}", path.display()))
}

/// Step a record up one version at a time to `RECORD_VERSION`.
#[must_use]
pub fn migrate(mut value: Value) -> Value {
    let mut version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
    while version < u64::from(RECORD_VERSION) {
        // 0 to 1: records gain a version; the fields added with it
        // (a ticket's close progress, a lane's `removed`, a project's
        // `closing`) arrive from their serde defaults, and no existing
        // field changes meaning.
        version += 1;
        if let Some(record) = value.as_object_mut() {
            record.insert("version".into(), version.into());
        }
    }
    value
}

/// Write a ticket record stamped with this build's version; one read
/// from a newer build is refused rather than written back without the
/// fields it carried. The caller's copy is left as it was.
pub fn write_ticket(path: &Path, t: &Ticket) -> Result<()> {
    if t.version > RECORD_VERSION {
        bail!(
            "ticket {} is version {}, written by a newer dispatch; update this one",
            t.id,
            t.version
        );
    }
    let stamped = Ticket {
        version: RECORD_VERSION,
        ..t.clone()
    };
    write_json(path, &stamped)
}

/// The same for a project's state.
pub fn write_project(path: &Path, ps: &ProjectState) -> Result<()> {
    if ps.version > RECORD_VERSION {
        bail!(
            "project {} is version {}, written by a newer dispatch; update this one",
            ps.name,
            ps.version
        );
    }
    let stamped = ProjectState {
        version: RECORD_VERSION,
        ..ps.clone()
    };
    write_json(path, &stamped)
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

    /// A ticket as records were written before they carried a version.
    const TICKET_V0: &str = r#"{
  "id": "a1b2c3d4",
  "project": "Orchard",
  "source": {
    "kind": "github",
    "identity": "k3/orchard#42",
    "number": 42,
    "title": "Asset report column missing",
    "body": "",
    "url": null,
    "labels": ["area:backend"],
    "taken_at_ms": 1000,
    "pull_requests": []
  },
  "pipeline_fingerprint": "f00d",
  "pipeline_file": "/d/tickets/a1b2c3d4/pipeline.toml",
  "lanes": [
    {
      "name": "backend",
      "worktree": "/wt/a1b2c3d4/orchard-backend",
      "branch": "dispatch/42-asset-report-column-missing",
      "project": null,
      "chosen": true,
      "setup_done": false,
      "base_sha": "base0000"
    }
  ],
  "tree": "/wt/a1b2c3d4",
  "stage": 1,
  "attempts": [],
  "decisions": [],
  "ledger": [],
  "processes": [],
  "root_project": null,
  "rework": {},
  "state": "parked",
  "reason": "parked by hand at decision lanes",
  "created_ms": 1000,
  "updated_ms": 2000
}"#;

    const PROJECT_V0: &str = r#"{
  "name": "Orchard",
  "space": "space-1",
  "set": "set-1",
  "queue": ["a1b2c3d4"],
  "shown": [["a1b2c3d4", "s-1"]]
}"#;

    #[test]
    fn a_record_without_a_version_migrates_to_this_one_and_is_written_back_as_it() {
        let dir = tempfile::tempdir().unwrap();
        let ticket = dir.path().join("t.json");
        let project = dir.path().join("p.json");
        fs::write(&ticket, TICKET_V0).unwrap();
        fs::write(&project, PROJECT_V0).unwrap();
        let t = read_ticket(&ticket).unwrap();
        assert_eq!(t.version, RECORD_VERSION);
        assert_eq!(t.close, crate::ticket::CloseProgress::default());
        assert!(!t.lanes[0].removed);
        assert!(
            matches!(&t.state, crate::ticket::TicketState::Parked { reason } if reason.contains("lanes"))
        );
        let ps = read_project(&project).unwrap();
        assert_eq!(ps.version, RECORD_VERSION);
        assert!(ps.closing.is_empty());
        assert_eq!(ps.queue, ["a1b2c3d4"]);
        write_ticket(&ticket, &t).unwrap();
        write_project(&project, &ps).unwrap();
        for path in [&ticket, &project] {
            let value: Value = read_json(path).unwrap();
            assert_eq!(value["version"], RECORD_VERSION);
        }
    }

    #[test]
    fn a_record_from_a_newer_dispatch_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        let newer = TICKET_V0.replacen(
            '{',
            &format!("{{\n  \"version\": {},", RECORD_VERSION + 1),
            1,
        );
        fs::write(&path, &newer).unwrap();
        let e = read_ticket(&path).unwrap_err();
        assert!(e.to_string().contains("newer dispatch"), "{e:#}");
        assert_eq!(fs::read_to_string(&path).unwrap(), newer);
        let t = serde_json::from_str::<Ticket>(&newer).unwrap();
        assert!(write_ticket(&path, &t).is_err(), "never written back");
        assert_eq!(fs::read_to_string(&path).unwrap(), newer);
    }

    #[test]
    fn a_closing_ticket_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        let mut t: Ticket = serde_json::from_str(TICKET_V0).unwrap();
        t.state = crate::ticket::TicketState::Closing {
            reason: "closed by hand".into(),
        };
        t.close.decisions_cancelled = true;
        t.close.trees_kept = Some("has changes".into());
        t.lanes[0].removed = true;
        write_ticket(&path, &t).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains(r#""state": "closing""#), "{text}");
        let stamped = Ticket {
            version: RECORD_VERSION,
            ..t
        };
        assert_eq!(read_ticket(&path).unwrap(), stamped);
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
        assert!(
            data.ticket_files().unwrap().is_empty(),
            "a fresh data directory has no tickets"
        );
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
