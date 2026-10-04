//! The data directory: where records live and how they are written.
//! Every write is a temp file, fsync, `.bak` of the previous version, and
//! a rename, under one writer lock shared by the runner and the commands.
//! The lock covers a whole read-modify-write, so the runner's pass and a
//! command from the terminal never interleave on a record. Slow external
//! work inside a step (a tree removal) runs with the lock let go, and the
//! record is read again under the lock before the next write.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::ticket::{ProjectState, Ticket};

/// The format of a ticket or project record this build reads and
/// writes. A record carries it as `version`; one written before records
/// carried a version reads as 0 and is brought up by `migrate`. A
/// record above it was written by a newer `dispatch` and is refused
/// both ways, so this build never drops fields it does not know.
pub const RECORD_VERSION: u32 = 7;

/// A lock file held while this lives: the writer lock, the runner's
/// claim, or a ticket's close.
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
fn default_worktrees_dir() -> Option<PathBuf> {
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

    /// Dispatch's own clone of a lane's repository, for a lane with a
    /// repository of its own.
    #[must_use]
    pub fn lane_repo_dir(&self, project: &str, lane: &str) -> PathBuf {
        self.repo_dir(&format!("{project}@{lane}"))
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
    fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// The directory's own settings; absent or unreadable reads as
    /// defaults.
    #[must_use]
    fn settings(&self) -> Settings {
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

    /// Take the writer lock, waiting for whoever holds it. Every
    /// read-modify-write of a record happens under it, from the runner
    /// and the commands alike, so two of them never interleave.
    ///
    /// The holder writes `pid <n>: <command>` into the lock file, and a
    /// wait over two seconds logs one line naming it, to stderr, so a
    /// command stuck behind the runner says why.
    pub fn lock(&self) -> Result<Lock> {
        self.lock_noting(Duration::from_secs(2), |holder| {
            log::info!("waiting for the writer lock (held by {holder})");
        })
    }

    /// `lock`, calling `note` with the holder's line once if the wait
    /// passes `after`. The holder is read when the note is made, not
    /// when the wait begins: the runner takes the lock again at every
    /// ticket, so an earlier read would often name a holder long gone.
    /// One blocking `lock`, never a `try_lock` loop, which would starve
    /// behind a runner that lets go and takes it again in microseconds.
    fn lock_noting(
        &self,
        after: Duration,
        note: impl FnOnce(&str) + Send + 'static,
    ) -> Result<Lock> {
        let path = self.root.join("lock");
        let file = open_lock(&path)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                let (taken, waiting) = std::sync::mpsc::channel::<()>();
                let noter = std::thread::spawn(move || {
                    if let Err(RecvTimeoutError::Timeout) = waiting.recv_timeout(after) {
                        // A handle of its own: the lock's is write-only.
                        let text = fs::read_to_string(&path).unwrap_or_default();
                        let holder = text.lines().next().unwrap_or("").trim();
                        note(if holder.is_empty() {
                            "an unknown process"
                        } else {
                            holder
                        });
                    }
                });
                let locked = file.lock().context("take the writer lock");
                let _ = taken.send(());
                let _ = noter.join();
                locked?;
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(e).context("take the writer lock");
            }
        }
        note_holder(&file);
        Ok(Lock(file))
    }

    /// The close of ticket `id`, held by one finisher at a time, or
    /// `None` at once when another holds it. A close lets the writer
    /// lock go while it removes trees, and this keeps a second finisher
    /// (a pass, another `dispatch close`) out of that gap. A lock file
    /// in the ticket's directory, not a record; the kernel drops it with
    /// the process.
    pub fn claim_close(&self, id: &str) -> Result<Option<Lock>> {
        try_lock_at(&self.ticket_dir(id).join("closing.lock"))
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
        let path = self.root.join("runner.lock");
        try_lock_at(&path)?
            .with_context(|| format!("another dispatch run holds {}", path.display()))
    }
}

/// The lock file at `path`, created with its directory if missing.
/// Never truncated on open: the writer lock's holder owns its contents
/// (`note_holder`), and a waiter reads them.
fn open_lock(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))
}

/// The lock at `path` taken without waiting, or `None` when another
/// holds it.
fn try_lock_at(path: &Path) -> Result<Option<Lock>> {
    let file = open_lock(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("lock {}", path.display()))
        }
    }
}

/// Who holds the writer lock, written into the lock file by its holder:
/// `pid <n>: <command>`. `flock` is advisory, so the contents are free.
/// No fsync: it is a hint for a waiter, and a failed write loses only
/// that.
fn note_holder(file: &File) {
    use std::os::unix::fs::FileExt as _;
    let mut args = std::env::args();
    let program = args
        .next()
        .map(|a| {
            Path::new(&a)
                .file_name()
                .map_or(a.clone(), |n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_default();
    let command: Vec<String> = std::iter::once(program).chain(args).collect();
    let line = format!("pid {}: {}\n", std::process::id(), command.join(" "));
    let _ = file.set_len(0);
    let _ = file.write_all_at(line.as_bytes(), 0);
}

/// Whether `atomic_write` flushes to the device. Only
/// `skip_fsync_for_tests` clears it; an atomic because test binaries
/// write from many threads, and `Relaxed` because nothing else
/// synchronises on it.
static FLUSH: AtomicBool = AtomicBool::new(true);

/// Tests only: `atomic_write` skips its two device flushes from now on,
/// in this process. The temp file, `.bak` link and rename still run, so
/// the write path and its fallback read stay covered; only durability
/// across a power cut is given up, which no test exercises.
pub fn skip_fsync_for_tests() {
    FLUSH.store(false, Ordering::Relaxed);
}

/// Write `bytes` to `path` so a crash leaves either the old file or the
/// new one, never a torn one and never none: temp file, fsync, the
/// previous version linked as `.bak` (the primary stays in place until
/// the rename replaces it), rename, directory fsync. A test process can
/// skip both fsyncs with `skip_fsync_for_tests`.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("a path with a parent")?;
    fs::create_dir_all(dir)?;
    let tmp = with_suffix(path, "tmp");
    {
        let mut file = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        file.write_all(bytes)?;
        if FLUSH.load(Ordering::Relaxed) {
            file.sync_all()?;
        }
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
    if FLUSH.load(Ordering::Relaxed)
        && let Ok(d) = File::open(dir)
    {
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
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
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
        //
        // 1 to 2: a ledger operation gains `settled`, which before was
        // read off recovery's verdict in `error`.
        //
        // 2 to 3: a lane gains `pushed`, absent until a refresh pushes,
        // which its serde default gives.
        //
        // 3 to 4: a code review attempt gains `carried_from` and
        // `rework`, and a lane's `refreshed` gains `commits`, `notes` and
        // `at_ms`, all from their serde defaults. A review attempt's
        // send-back note now lives on the attempt, so a build that would
        // drop those fields must refuse the record; an attempt started
        // before still finds its note on the ticket.
        //
        // 4 to 5: a code review attempt gains `rewrite`, absent until it
        // rewrites its commits, which its serde default gives. Nothing is
        // transformed; a build that would drop it on its next write must
        // refuse the record.
        //
        // 5 to 6: a review round gains `dirty_since_ms`, absent until a
        // pass finds the tree dirty after the response, which its serde
        // default gives. Nothing is transformed; a build that would drop
        // it on its next write must refuse the record.
        //
        // 6 to 7: an attempt and a review round gain `nudges`, empty
        // until a dirty stop is nudged, which their serde default gives.
        // Nothing is transformed; a build that would drop them on its
        // next write must refuse the record, or a restart would send a
        // nudge already sent.
        if version == 1 {
            settle_from_verdicts(&mut value);
        }
        version += 1;
        if let Some(record) = value.as_object_mut() {
            record.insert("version".into(), version.into());
        }
    }
    value
}

/// Every ledger operation whose `error` is one of recovery's verdicts
/// marked settled. A project record has no ledger and is left alone.
fn settle_from_verdicts(value: &mut Value) {
    use crate::recover::{HARMLESS, INTERRUPTED, LOST, NOT_REPEATED, REMOVED};
    let Some(ledger) = value.get_mut("ledger").and_then(Value::as_array_mut) else {
        return;
    };
    for op in ledger.iter_mut().filter_map(Value::as_object_mut) {
        let verdict = op
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|e| [LOST, INTERRUPTED, REMOVED, NOT_REPEATED, HARMLESS].contains(&e));
        op.insert("settled".into(), verdict.into());
    }
}

/// Write a ticket record stamped with this build's version; one read
/// from a newer build is refused rather than written back without the
/// fields it carried. The caller's copy is left as it was.
pub fn write_ticket(path: &Path, t: &Ticket) -> Result<()> {
    refuse_newer("ticket", &t.id, t.version)?;
    let stamped = Ticket {
        version: RECORD_VERSION,
        ..t.clone()
    };
    write_json(path, &stamped)
}

/// Write a ticket record and log what the write changes. The record it
/// replaces is read and diffed against `t` (`events::between`), the
/// events are appended and synced, and only then is the record written,
/// so a crash between the two repeats a transition rather than losing
/// it. A record write that fails after the append is withdrawn with a
/// `void` event naming the appended seqs; if that append fails too, the
/// record's error is still the one returned and the events stand
/// unwithdrawn. An append that fails refuses the write, since the
/// transition would otherwise never be logged. The caller holds the
/// writer lock.
pub fn write_ticket_logged(data: &DataDir, t: &Ticket, at_ms: u64) -> Result<()> {
    refuse_newer("ticket", &t.id, t.version)?;
    let path = data.ticket_file(&t.id);
    let old = if record_exists(&path) {
        Some(read_ticket(&path)?)
    } else {
        None
    };
    let mut events =
        crate::events::between(old.as_ref(), t, at_ms, &|| crate::events::stage_names(t));
    let log = crate::events::log_path(data);
    crate::events::append(&log, &mut events)?;
    let written = write_ticket(&path, t);
    if let Err(err) = &written
        && let Some(first) = events.first()
    {
        let seqs = events.iter().map(|ev| ev.seq).collect();
        if let Err(void) = crate::events::append_void(&log, first, seqs, &format!("{err:#}")) {
            log::error!(
                "ticket {}: its write failed and the events for it could not be withdrawn: {void:#}",
                t.id
            );
        }
    }
    written
}

/// The same for a project's state.
pub fn write_project(path: &Path, ps: &ProjectState) -> Result<()> {
    refuse_newer("project", &ps.name, ps.version)?;
    let stamped = ProjectState {
        version: RECORD_VERSION,
        ..ps.clone()
    };
    write_json(path, &stamped)
}

/// A record a newer dispatch wrote is never written over: this one would
/// drop the fields it does not know.
fn refuse_newer(what: &str, name: &str, version: u32) -> Result<()> {
    if version > RECORD_VERSION {
        bail!("{what} {name} is version {version}, written by a newer dispatch; update this one");
    }
    Ok(())
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
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
    fn a_waiter_notes_the_holder_once_after_the_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let held = data.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let waiter = {
            let data = data.clone();
            std::thread::spawn(move || {
                data.lock_noting(Duration::from_millis(50), move |holder| {
                    tx.send(holder.to_owned()).unwrap();
                })
                .unwrap()
            })
        };
        let holder = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            holder.starts_with(&format!("pid {}: ", std::process::id())),
            "{holder}"
        );
        drop(held);
        let lock = waiter.join().unwrap();
        // The sender went with the closure: one note, no more.
        assert!(rx.recv().is_err());
        drop(lock);
    }

    #[test]
    fn an_uncontended_lock_notes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        let lock = data
            .lock_noting(Duration::ZERO, move |holder| {
                tx.send(holder.to_owned()).unwrap();
            })
            .unwrap();
        assert!(rx.recv().is_err());
        drop(lock);
    }

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
    fn the_backup_holds_the_previous_write_and_counts_as_the_record() {
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
    fn an_older_ledger_reads_recoverys_verdicts_as_settled() {
        let op = |error: &str| {
            format!(
                r#"{{"op": "o", "kind": "k", "class": "creation", "attempt": null, "sent_ms": 1, "reply": null, "error": {error}}}"#
            )
        };
        let ledger = format!(
            r#""ledger": [{}, {}, {}],"#,
            op(&format!("\"{}\"", crate::recover::INTERRUPTED)),
            op("\"socket closed\""),
            op("null")
        );
        for version in ["", "\"version\": 1,"] {
            let text = TICKET_V0.replacen(r#""ledger": [],"#, &ledger, 1).replacen(
                '{',
                &format!("{{{version}"),
                1,
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("t.json");
            fs::write(&path, text).unwrap();
            let t = read_ticket(&path).unwrap();
            let settled: Vec<bool> = t.ledger.iter().map(|o| o.settled).collect();
            assert_eq!(settled, [true, false, false], "from {version:?}");
        }
    }

    #[test]
    fn a_version_three_record_migrates_to_four_and_keeps_its_note_on_the_ticket() {
        let attempt = r#"{"stage": "review-code", "n": 1, "context": "backend", "kind": "review", "state": "failed", "reason": "lint", "project": null, "session": null, "run": null, "artifacts": {}, "settle": {}, "stop_at_ms": null, "head": null, "started_ms": 1000, "ended_ms": 2000}"#;
        let text = TICKET_V0
            .replacen('{', "{\n  \"version\": 3,", 1)
            .replacen(
                r#""attempts": [],"#,
                &format!(r#""attempts": [{attempt}],"#),
                1,
            )
            .replacen(
                r#""rework": {},"#,
                r#""rework": {"review-code/backend": "rename tmp"},"#,
                1,
            )
            .replacen(
                r#""base_sha": "base0000""#,
                r#""base_sha": "main0002", "refreshed": {"from": "base0000", "to": "main0002"}"#,
                1,
            );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        fs::write(&path, text).unwrap();
        let mut t = read_ticket(&path).unwrap();
        assert_eq!(t.version, RECORD_VERSION);
        assert_eq!(t.rework["review-code/backend"], "rename tmp");
        let a = &t.attempts[0];
        assert_eq!((a.carried_from.clone(), a.rework.clone()), (None, None));
        let moved = t.lanes[0].refreshed.clone().unwrap();
        assert_eq!(
            (moved.commits, moved.notes.clone(), moved.at_ms),
            (false, None, 0)
        );
        t.attempts[0].carried_from = Some(("review-code".into(), 0));
        t.attempts[0].rework = Some("start over".into());
        t.lanes[0].refreshed = Some(crate::ticket::Refreshed {
            commits: true,
            notes: Some("/n.md".into()),
            at_ms: 5,
            ..moved
        });
        write_ticket(&path, &t).unwrap();
        assert_eq!(read_ticket(&path).unwrap(), t);
    }

    #[test]
    fn a_version_four_record_migrates_to_five_without_a_rewrite() {
        let attempt = r#"{"stage": "review-code", "n": 1, "context": "backend", "kind": "review", "state": "complete", "project": null, "session": null, "run": null, "artifacts": {}, "settle": {}, "stop_at_ms": null, "head": "fix00002", "carried_from": null, "rework": null, "started_ms": 1000, "ended_ms": 2000}"#;
        let text = TICKET_V0.replacen('{', "{\n  \"version\": 4,", 1).replacen(
            r#""attempts": [],"#,
            &format!(r#""attempts": [{attempt}],"#),
            1,
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        fs::write(&path, text).unwrap();
        let mut t = read_ticket(&path).unwrap();
        assert_eq!(t.attempts[0].rewrite, None);
        t.attempts[0].rewrite = Some(crate::ticket::Rewrite {
            mode: crate::history::Commits::Fold,
            before: "fix00002".into(),
            after: Some("fold0001".into()),
            from: 4,
            to: 2,
            skipped: None,
            at_ms: 3000,
        });
        write_ticket(&path, &t).unwrap();
        let written: Value = read_json(&path).unwrap();
        assert_eq!(written["version"], RECORD_VERSION);
        assert_eq!(written["attempts"][0]["rewrite"]["mode"], "fold");
        assert_eq!(read_ticket(&path).unwrap(), t);
    }

    #[test]
    fn a_version_five_record_migrates_to_six_and_reads_no_dirty_clock() {
        let round = r#"{"n": 1, "base": "base0000", "head": "head0001", "reviewers": [], "round_state": "fixing", "feedback": null, "open_points": 2, "fix_authorised": true, "implementer": "s-9", "response": "/r.md", "head_after": null, "stop_at_ms": 1500, "polls_since_stop": 0, "settle": null, "dirty_polls": 4, "started_ms": 1000, "ended_ms": null}"#;
        let attempt = format!(
            r#"{{"stage": "review-code", "n": 1, "context": "backend", "kind": "review", "state": "running", "project": null, "session": null, "run": null, "artifacts": {{}}, "settle": {{}}, "stop_at_ms": null, "head": null, "carried_from": null, "rework": null, "rounds": [{round}], "started_ms": 1000, "ended_ms": null}}"#
        );
        let text = TICKET_V0.replacen('{', "{\n  \"version\": 5,", 1).replacen(
            r#""attempts": [],"#,
            &format!(r#""attempts": [{attempt}],"#),
            1,
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        fs::write(&path, text).unwrap();
        let mut t = read_ticket(&path).unwrap();
        assert_eq!(t.version, RECORD_VERSION);
        let rm = &t.attempts[0].rounds[0];
        assert_eq!((rm.dirty_since_ms, rm.dirty_polls), (None, 4));
        t.attempts[0].rounds[0].dirty_since_ms = Some(5_000);
        write_ticket(&path, &t).unwrap();
        let written: Value = read_json(&path).unwrap();
        assert_eq!(written["version"], RECORD_VERSION);
        assert_eq!(
            read_ticket(&path).unwrap().attempts[0].rounds[0].dirty_since_ms,
            Some(5_000)
        );
    }

    #[test]
    fn a_version_six_record_migrates_to_seven_with_no_nudges() {
        let round = r#"{"n": 1, "base": "base0000", "head": "head0001", "reviewers": [], "round_state": "fixing", "feedback": null, "open_points": 2, "fix_authorised": true, "implementer": "s-9", "response": "/r.md", "head_after": null, "stop_at_ms": 1500, "polls_since_stop": 0, "settle": null, "dirty_polls": 0, "dirty_since_ms": null, "started_ms": 1000, "ended_ms": null}"#;
        let attempt = format!(
            r#"{{"stage": "review-code", "n": 1, "context": "backend", "kind": "review", "state": "running", "project": null, "session": null, "run": null, "artifacts": {{}}, "settle": {{}}, "stop_at_ms": null, "head": null, "carried_from": null, "rework": null, "rounds": [{round}], "started_ms": 1000, "ended_ms": null}}"#
        );
        let text = TICKET_V0.replacen('{', "{\n  \"version\": 6,", 1).replacen(
            r#""attempts": [],"#,
            &format!(r#""attempts": [{attempt}],"#),
            1,
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        fs::write(&path, text).unwrap();
        let mut t = read_ticket(&path).unwrap();
        assert_eq!(t.version, RECORD_VERSION);
        assert!(t.attempts[0].nudges.is_empty());
        assert!(t.attempts[0].rounds[0].nudges.is_empty());
        t.attempts[0].nudges.push(4_000);
        t.attempts[0].rounds[0].nudges.push(5_000);
        write_ticket(&path, &t).unwrap();
        let written: Value = read_json(&path).unwrap();
        assert_eq!(written["version"], 7);
        assert_eq!(written["attempts"][0]["nudges"][0], 4_000);
        assert_eq!(read_ticket(&path).unwrap(), t);
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

    /// A ticket record for the logged write's tests, with no pipeline
    /// copy to name its stages.
    fn logged_ticket() -> Ticket {
        Ticket {
            version: RECORD_VERSION,
            id: "t1".into(),
            ..serde_json::from_str(TICKET_V0).unwrap()
        }
    }

    #[test]
    fn a_logged_write_refused_as_newer_appends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let t = Ticket {
            version: RECORD_VERSION + 1,
            ..logged_ticket()
        };
        assert!(write_ticket_logged(&data, &t, 1).is_err());
        assert!(!crate::events::log_path(&data).exists());
        assert!(!data.ticket_file("t1").exists());
    }

    #[test]
    fn a_logged_write_that_fails_withdraws_its_events() {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::new(dir.path());
        let mut t = logged_ticket();
        write_ticket_logged(&data, &t, 1).unwrap();
        // A second write leaves a `.bak`, which still reads as the
        // record once the primary is broken below.
        write_ticket_logged(&data, &t, 1).unwrap();
        t.decisions.push(crate::ticket::Decision {
            id: "d1".into(),
            stage: "plan".into(),
            name: "finalize".into(),
            kind: crate::ticket::DecisionKind::Permission,
            question: "Finalize it?".into(),
            options: vec!["finalize".into()],
            recommendation: None,
            attempt: None,
            state: crate::ticket::DecisionState::Pending,
            made_ms: 2,
        });
        // The record's place is a directory: the old record reads from
        // its backup, and the write fails after the append.
        let path = data.ticket_file("t1");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("x"), "x").unwrap();
        assert!(write_ticket_logged(&data, &t, 2).is_err());
        let events = crate::events::read_since(&crate::events::log_path(&data), 0).unwrap();
        let kinds: Vec<_> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, ["taken", "decision", "void"]);
        assert_eq!(events[2].voids, [events[1].seq]);
        assert_eq!(
            crate::events::withdrawn(&events)
                .into_iter()
                .collect::<Vec<_>>(),
            [events[1].seq]
        );
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
    fn writes_under_the_lock_are_listed_as_tickets() {
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
