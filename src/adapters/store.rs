//! JSON store under the private data directory. See the design's
//! "Durable store": one file per project, atomic writes with `.bak`,
//! an OS-held single-writer lock, validation on load.
//!
//! Layout under the data directory:
//!
//! ```text
//! <dir>/lock                              advisory lock, held while running
//! <dir>/projects/<project uuid>.json      current record
//! <dir>/projects/<project uuid>.json.bak  previous record
//! <dir>/projects/<project uuid>.json.tmp  write in flight (never read)
//! ```

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use uuid::Uuid;

use crate::core::{ProjectId, SCHEMA_VERSION, Workspace};
use crate::ports::store::{Loaded, Store, StoreError};

/// Store backed by JSON files. Construct with [`JsonStore::new`], then
/// call [`Store::lock`] before saving; saves are refused otherwise.
#[derive(Debug)]
pub struct JsonStore {
    dir: PathBuf,
    /// The lock lives exactly as long as this open file: `flock` releases
    /// when the descriptor closes, which also happens when the process
    /// dies, so a crash never leaves a stale lock. `None` until `lock()`
    /// succeeds.
    lock: Option<File>,
}

impl JsonStore {
    /// A store rooted at `dir`. Nothing is created until the first lock
    /// or save.
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, lock: None }
    }

    /// The platform data directory for Switchboard:
    /// `~/Library/Application Support/Switchboard` on macOS.
    pub fn default_dir() -> Result<PathBuf, StoreError> {
        ProjectDirs::from("", "", "Switchboard")
            .map(|d| d.data_dir().to_path_buf())
            .ok_or_else(|| StoreError::Io("no home directory for the data dir".into()))
    }

    fn projects_dir(&self) -> PathBuf {
        self.dir.join("projects")
    }

    fn lock_path(&self) -> PathBuf {
        self.dir.join("lock")
    }

    fn record_path(&self, id: ProjectId) -> PathBuf {
        self.projects_dir().join(format!("{}.json", id.0))
    }

    fn backup_path(path: &Path) -> PathBuf {
        with_suffix(path, "bak")
    }

    fn temp_path(path: &Path) -> PathBuf {
        with_suffix(path, "tmp")
    }

    /// Read one record, preferring the current file and falling back to
    /// `.bak`. The notice is `Some` whenever the current file was
    /// unusable, so the UI can say so even when recovery worked.
    fn load_record(path: &Path, expected: ProjectId) -> (Option<Workspace>, Option<StoreError>) {
        let detail = match read_record(path, expected) {
            Ok(ws) => return (Some(ws), None),
            Err(detail) => detail,
        };
        match read_record(&Self::backup_path(path), expected) {
            Ok(ws) => (
                Some(ws),
                Some(StoreError::Corrupt {
                    path: path.to_path_buf(),
                    recovered: true,
                    detail,
                }),
            ),
            Err(backup_detail) => (
                None,
                Some(StoreError::Corrupt {
                    path: path.to_path_buf(),
                    recovered: false,
                    detail: format!("{detail}; backup: {backup_detail}"),
                }),
            ),
        }
    }
}

impl Store for JsonStore {
    fn lock(&mut self) -> Result<bool, StoreError> {
        if self.lock.is_some() {
            return Ok(true);
        }
        ensure_dir(&self.dir)?;
        let path = self.lock_path();
        let file = open_private(&path, false)?;
        match file.try_lock() {
            Ok(()) => {
                self.lock = Some(file);
                Ok(true)
            }
            Err(TryLockError::WouldBlock) => Ok(false),
            Err(TryLockError::Error(e)) => Err(io_err("lock", &path, &e)),
        }
    }

    fn load_all(&self) -> Result<Loaded, StoreError> {
        let dir = self.projects_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::default()),
            Err(e) => return Err(io_err("read", &dir, &e)),
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();

        let mut loaded = Loaded::default();
        for path in paths {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let Ok(id) = Uuid::parse_str(stem) else {
                loaded.notices.push(StoreError::Corrupt {
                    path,
                    recovered: false,
                    detail: "file name is not a project id".into(),
                });
                continue;
            };
            let (workspace, notice) = Self::load_record(&path, ProjectId(id));
            loaded.workspaces.extend(workspace);
            loaded.notices.extend(notice);
        }
        Ok(loaded)
    }

    fn save(&self, workspace: &Workspace) -> Result<(), StoreError> {
        if self.lock.is_none() {
            return Err(StoreError::Locked);
        }
        if workspace.schema_version > SCHEMA_VERSION {
            return Err(StoreError::Io(format!(
                "refusing to write schema version {} (this build understands {SCHEMA_VERSION})",
                workspace.schema_version
            )));
        }
        let json = serde_json::to_vec_pretty(workspace)
            .map_err(|e| StoreError::Io(format!("serialize: {e}")))?;

        let dir = self.projects_dir();
        ensure_dir(&dir)?;
        let path = self.record_path(workspace.project.id);
        let tmp = Self::temp_path(&path);
        let bak = Self::backup_path(&path);

        // The whole record goes to a temp file that is fsynced before it
        // takes the real name, so a crash mid-write can only leave a
        // stray `.tmp`, which loads ignore and the next save overwrites.
        let mut file = open_private(&tmp, true)?;
        file.write_all(&json)
            .and_then(|()| file.sync_all())
            .map_err(|e| io_err("write", &tmp, &e))?;
        drop(file);

        match fs::rename(&path, &bak) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("back up", &path, &e)),
        }
        fs::rename(&tmp, &path).map_err(|e| io_err("rename", &tmp, &e))?;
        sync_dir(&dir)
    }

    fn delete(&self, id: ProjectId) -> Result<(), StoreError> {
        if self.lock.is_none() {
            return Err(StoreError::Locked);
        }
        let dir = self.projects_dir();
        if !dir.exists() {
            return Ok(());
        }
        let path = self.record_path(id);
        remove_if_present(&path)?;
        remove_if_present(&Self::backup_path(&path))?;
        remove_if_present(&Self::temp_path(&path))?;
        sync_dir(&dir)
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.clone()
    }
}

/// Parse a raw JSON document into the current `Workspace` shape. Older
/// schema versions are upgraded here; add a step per version bump.
///
/// # Errors
/// A version this build does not understand, or a document that does not
/// match its declared version.
pub fn migrate(value: serde_json::Value) -> Result<Workspace, String> {
    let version = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "missing schema_version".to_string())?;
    match version {
        1 => serde_json::from_value(value).map_err(|e| format!("schema v1: {e}")),
        v if v > u64::from(SCHEMA_VERSION) => Err(format!(
            "schema version {v} is newer than this build understands ({SCHEMA_VERSION})"
        )),
        v => Err(format!("unknown schema version {v}")),
    }
}

/// Read and validate one file. Every failure is a string because the
/// caller turns it into a `Corrupt` notice.
fn read_record(path: &Path, expected: ProjectId) -> Result<Workspace, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let workspace = migrate(value)?;
    if workspace.project.id != expected {
        return Err(format!(
            "project id {} does not match file name",
            workspace.project.id.0
        ));
    }
    Ok(workspace)
}

/// `foo.json` -> `foo.json.<suffix>`; keeping `.json` in the name makes
/// the file's origin obvious in a directory listing.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

fn io_err(what: &str, path: &Path, e: &std::io::Error) -> StoreError {
    StoreError::Io(format!("{what} {}: {e}", path.display()))
}

/// Create `dir` (and parents) readable only by the owner.
fn ensure_dir(dir: &Path) -> Result<(), StoreError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    // `#[cfg(unix)]` compiles the block only on unix targets; elsewhere
    // the directory gets the platform default permissions.
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|e| io_err("create", dir, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // `mode` above is subject to the umask and skipped for a directory
        // that already existed, so set it explicitly as well.
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| io_err("chmod", dir, &e))?;
    }
    Ok(())
}

/// Open (creating if needed) a file readable and writable only by the
/// owner. `truncate` discards existing content.
fn open_private(path: &Path, truncate: bool) -> Result<File, StoreError> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|e| io_err("open", path, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| io_err("chmod", path, &e))?;
    }
    Ok(file)
}

/// Flush a directory's entries so a rename survives a power loss.
fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| io_err("sync", dir, &e))
}

fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err("remove", path, &e)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::core::{
        Activity, AgentKind, CardLayout, Launch, Project, RecordId, ResumeHandle, SessionKind,
        SessionRecord,
    };

    fn workspace(name: &str) -> Workspace {
        let id = ProjectId::new();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let session = |name: &str, kind: SessionKind, launch: Launch| SessionRecord {
            id: RecordId::new(),
            project: id,
            name: name.into(),
            kind,
            cwd: PathBuf::from("/tmp/proj"),
            launch,
            env_profile: None,
            created: now,
            last_seen: now,
            notes: String::new(),
            resume: None,
            autostart: false,
            layout: CardLayout::default(),
            activity: Activity::Unknown,
            last_event_at: None,
            last_exit: None,
            not_resumable: false,
            scrollback: None,
        };
        let mut agent = session(
            "claude",
            SessionKind::Agent(AgentKind::ClaudeCode),
            Launch::Argv(vec!["claude".into(), "--model".into(), "haiku".into()]),
        );
        agent.resume = Some(ResumeHandle::ClaudeCode {
            session_id: Uuid::new_v4(),
            transcript: Some(PathBuf::from("/tmp/transcript.jsonl")),
        });
        let mut shell = session("shell", SessionKind::Shell, Launch::Shell);
        shell.layout.order = 1;
        Workspace {
            schema_version: SCHEMA_VERSION,
            project: Project {
                id,
                name: name.into(),
                root: PathBuf::from("/tmp/proj"),
                tags: vec!["rust".into()],
                notes: "notes".into(),
                pinned: vec![PathBuf::from("README.md")],
                created: now,
                last_active: now,
            },
            sessions: vec![agent, shell],
        }
    }

    fn locked_store(dir: &Path) -> JsonStore {
        let mut store = JsonStore::new(dir.to_path_buf());
        assert_eq!(store.lock(), Ok(true));
        store
    }

    fn record_path(store: &JsonStore, ws: &Workspace) -> PathBuf {
        store.record_path(ws.project.id)
    }

    #[test]
    fn round_trips_a_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("alpha");
        store.save(&ws).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.workspaces, vec![ws]);
        assert!(loaded.notices.is_empty());
    }

    #[test]
    fn empty_store_loads_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path().join("missing"));
        assert_eq!(store.load_all().unwrap(), Loaded::default());
    }

    #[test]
    fn writes_are_refused_without_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path().to_path_buf());
        assert_eq!(store.save(&workspace("a")), Err(StoreError::Locked));
        assert_eq!(store.delete(ProjectId::new()), Err(StoreError::Locked));
    }

    #[test]
    fn save_refuses_newer_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let mut ws = workspace("a");
        ws.schema_version = SCHEMA_VERSION + 1;
        assert!(matches!(store.save(&ws), Err(StoreError::Io(_))));
        assert!(!record_path(&store, &ws).exists());
    }

    #[test]
    fn second_save_keeps_previous_in_bak() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let first = workspace("first");
        store.save(&first).unwrap();
        let mut second = first.clone();
        second.project.name = "second".into();
        store.save(&second).unwrap();

        let path = record_path(&store, &first);
        let current = read_record(&path, first.project.id).unwrap();
        let backup = read_record(&JsonStore::backup_path(&path), first.project.id).unwrap();
        assert_eq!(current, second);
        assert_eq!(backup, first);
        assert!(!JsonStore::temp_path(&path).exists());
    }

    #[test]
    fn truncated_file_recovers_from_bak_with_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("a");
        store.save(&ws).unwrap();
        store.save(&ws).unwrap();
        let path = record_path(&store, &ws);
        let full = fs::read(&path).unwrap();
        fs::write(&path, &full[..full.len() / 2]).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.workspaces, vec![ws]);
        assert_eq!(loaded.notices.len(), 1);
        assert!(matches!(
            &loaded.notices[0],
            StoreError::Corrupt { path: p, recovered: true, .. } if *p == path
        ));
        // Recovery is read-only: the damaged file is left for the user.
        assert_eq!(fs::read(&path).unwrap().len(), full.len() / 2);
    }

    #[test]
    fn corrupt_file_and_bak_are_skipped_with_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("a");
        store.save(&ws).unwrap();
        store.save(&ws).unwrap();
        let path = record_path(&store, &ws);
        fs::write(&path, b"{").unwrap();
        fs::write(JsonStore::backup_path(&path), b"").unwrap();

        let loaded = store.load_all().unwrap();
        assert!(loaded.workspaces.is_empty());
        assert_eq!(loaded.notices.len(), 1);
        assert!(matches!(
            &loaded.notices[0],
            StoreError::Corrupt {
                recovered: false,
                ..
            }
        ));
        assert!(path.exists(), "load never deletes");
    }

    #[test]
    fn newer_schema_version_is_corrupt_not_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("a");
        store.save(&ws).unwrap();
        let path = record_path(&store, &ws);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(99);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        let loaded = store.load_all().unwrap();
        assert!(loaded.workspaces.is_empty());
        assert!(matches!(
            &loaded.notices[0],
            StoreError::Corrupt { recovered: false, detail, .. } if detail.contains("99")
        ));
    }

    #[test]
    fn id_mismatch_is_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("a");
        store.save(&ws).unwrap();
        let other = store.record_path(ProjectId::new());
        fs::copy(record_path(&store, &ws), &other).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.workspaces, vec![ws]);
        assert_eq!(loaded.notices.len(), 1);
        assert!(matches!(
            &loaded.notices[0],
            StoreError::Corrupt { path: p, recovered: false, .. } if *p == other
        ));
    }

    #[test]
    fn lock_is_exclusive_across_stores() {
        let tmp = tempfile::tempdir().unwrap();
        let first = locked_store(tmp.path());
        let mut second = JsonStore::new(tmp.path().to_path_buf());
        assert_eq!(second.lock(), Ok(false));
        assert_eq!(second.save(&workspace("a")), Err(StoreError::Locked));

        drop(first);
        assert_eq!(second.lock(), Ok(true));
        assert_eq!(second.lock(), Ok(true), "locking again is a no-op");
        let mut third = JsonStore::new(tmp.path().to_path_buf());
        assert_eq!(third.lock(), Ok(false));
    }

    #[test]
    fn delete_removes_file_and_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("a");
        store.save(&ws).unwrap();
        store.save(&ws).unwrap();
        let path = record_path(&store, &ws);
        assert!(JsonStore::backup_path(&path).exists());

        store.delete(ws.project.id).unwrap();
        assert!(!path.exists());
        assert!(!JsonStore::backup_path(&path).exists());
        assert!(store.load_all().unwrap().workspaces.is_empty());
        store.delete(ws.project.id).unwrap();
    }

    #[test]
    fn stale_tmp_is_ignored_and_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("a");
        let path = record_path(&store, &ws);
        ensure_dir(&store.projects_dir()).unwrap();
        fs::write(JsonStore::temp_path(&path), b"half-written garbage").unwrap();

        assert_eq!(store.load_all().unwrap(), Loaded::default());
        store.save(&ws).unwrap();
        assert!(!JsonStore::temp_path(&path).exists());
        assert_eq!(store.load_all().unwrap().workspaces, vec![ws]);
    }

    #[test]
    fn written_file_declares_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let store = locked_store(tmp.path());
        let ws = workspace("a");
        store.save(&ws).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(record_path(&store, &ws)).unwrap()).unwrap();
        assert_eq!(value["schema_version"], serde_json::json!(SCHEMA_VERSION));
    }

    #[cfg(unix)]
    #[test]
    fn files_and_dirs_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested").join("Switchboard");
        let store = locked_store(&dir);
        let ws = workspace("a");
        store.save(&ws).unwrap();
        store.save(&ws).unwrap();

        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&store.projects_dir()), 0o700);
        assert_eq!(mode(&store.lock_path()), 0o600);
        let path = record_path(&store, &ws);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&JsonStore::backup_path(&path)), 0o600);
    }

    #[test]
    fn default_dir_ends_with_app_name() {
        let dir = JsonStore::default_dir().unwrap();
        assert!(dir.ends_with("Switchboard"), "{}", dir.display());
    }
}
