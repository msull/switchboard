//! The hosts this server is paired with, in `hosts.json` under its
//! config directory (`$SWITCHBOARD_MCP_DIR`, or
//! `~/.config/switchboard-mcp`). The directory is `0700` and the file
//! `0600`, written by temp file, fsync and rename. Each host's bearer
//! token is in it as plain text: the server is a headless child of an
//! agent session, where a Keychain prompt would hang the call. Any
//! process of this user can read it; the token never reaches a log
//! line, a tool result or `hosts` output.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file's format; one written by a newer build is refused.
pub const HOSTS_VERSION: u32 = 1;

/// The file's name under the config directory.
pub const HOSTS_FILE: &str = "hosts.json";

/// The lock file beside it, held while the hosts are changed.
const LOCK_FILE: &str = "hosts.lock";

/// The variable naming the config directory.
pub const DIR_VAR: &str = "SWITCHBOARD_MCP_DIR";

/// One paired host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// What the agent calls it in every tool's `host`.
    pub name: String,
    /// Where it answers, `host:port`, the first tried first.
    pub addresses: Vec<String>,
    pub server_id: String,
    /// The key the connection is pinned to.
    pub spki_sha256: String,
    /// The bearer token. Never printed.
    pub token: String,
    /// `read` or `control`, as the host granted.
    pub scope: String,
    pub paired_ms: u64,
    /// The host refused the token or showed another key: pair again.
    #[serde(default)]
    pub needs_pairing: bool,
}

/// The whole file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hosts {
    pub version: u32,
    pub hosts: Vec<Host>,
}

impl Hosts {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Host> {
        self.hosts.iter().find(|h| h.name == name)
    }
}

/// The config directory as the environment names it.
pub fn default_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os(DIR_VAR).filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| format!("no HOME; set {DIR_VAR}"))?;
    Ok(PathBuf::from(home).join(".config").join("switchboard-mcp"))
}

/// The hosts in `dir`; none when the file is absent.
pub fn load(dir: &Path) -> Result<Hosts, String> {
    let path = dir.join(HOSTS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Hosts {
                version: HOSTS_VERSION,
                hosts: Vec::new(),
            });
        }
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    let hosts: Hosts =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    if hosts.version > HOSTS_VERSION {
        return Err(format!(
            "{} is version {}, written by a newer switchboard-mcp",
            path.display(),
            hosts.version
        ));
    }
    Ok(hosts)
}

/// Write `hosts` to `dir`, making the directory `0700` and the file
/// `0600`, through a temporary file that is synced and renamed over.
pub fn save(dir: &Path, hosts: &Hosts) -> Result<(), String> {
    if hosts.version > HOSTS_VERSION {
        return Err("a newer switchboard-mcp wrote this file; update this one".into());
    }
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("make {}: {e}", dir.display()))?;
    // A directory made earlier by hand keeps its mode otherwise.
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod {}: {e}", dir.display()))?;
    let stamped = Hosts {
        version: HOSTS_VERSION,
        hosts: hosts.hosts.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&stamped).map_err(|e| e.to_string())?;
    let path = dir.join(HOSTS_FILE);
    let tmp = dir.join(format!(".{HOSTS_FILE}.{}.tmp", std::process::id()));
    let written = (|| -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, &path)?;
        // The rename is durable once the directory is.
        File::open(dir)?.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(format!("write {}: {e}", path.display()));
    }
    Ok(())
}

/// Run `change` on the hosts in `dir` while holding an exclusive lock
/// on a file beside them, so `pair`, `forget` and a tool's mark made in
/// other processes never read the file, each change it, and lose one
/// write. The hosts are saved when `change` says they changed.
fn locked_update(dir: &Path, change: impl FnOnce(&mut Hosts) -> bool) -> Result<(), String> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("make {}: {e}", dir.display()))?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(LOCK_FILE))
        .map_err(|e| format!("open {LOCK_FILE}: {e}"))?;
    // Released when `lock` drops.
    lock.lock().map_err(|e| format!("lock {LOCK_FILE}: {e}"))?;
    let mut hosts = load(dir)?;
    if change(&mut hosts) {
        save(dir, &hosts)?;
    }
    Ok(())
}

/// Put `host` in, replacing one of the same name.
pub fn upsert(dir: &Path, host: Host) -> Result<(), String> {
    locked_update(dir, |hosts| {
        hosts.hosts.retain(|h| h.name != host.name);
        hosts.hosts.push(host);
        true
    })
}

/// Mark `name` as needing pairing again, if it is still paired with
/// `token`, the one that failed. A call that began before the owner
/// paired again fails with the old token and must not mark the new
/// pairing; nothing happens when the host is gone or its token changed.
pub fn mark_needs_pairing(dir: &Path, name: &str, token: &str) -> Result<(), String> {
    locked_update(dir, |hosts| {
        match hosts
            .hosts
            .iter_mut()
            .find(|h| h.name == name && h.token == token && !h.needs_pairing)
        {
            Some(h) => {
                h.needs_pairing = true;
                true
            }
            None => false,
        }
    })
}

/// Drop `name`; whether it was there.
pub fn forget(dir: &Path, name: &str) -> Result<bool, String> {
    let mut gone = false;
    locked_update(dir, |hosts| {
        let before = hosts.hosts.len();
        hosts.hosts.retain(|h| h.name != name);
        gone = hosts.hosts.len() != before;
        gone
    })?;
    Ok(gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(name: &str) -> Host {
        Host {
            name: name.into(),
            addresses: vec!["127.0.0.1:43187".into()],
            server_id: "s".into(),
            spki_sha256: "ab".repeat(32),
            token: "tok".into(),
            scope: "control".into(),
            paired_ms: 1,
            needs_pairing: false,
        }
    }

    #[test]
    fn the_file_is_private_and_round_trips() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("mcp");
        upsert(&dir, host("box")).unwrap();
        upsert(&dir, host("box")).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(HOSTS_FILE)), 0o600);
        let hosts = load(&dir).unwrap();
        assert_eq!(hosts.hosts, vec![host("box")]);
        mark_needs_pairing(&dir, "box", "old").unwrap();
        assert!(
            !load(&dir).unwrap().get("box").unwrap().needs_pairing,
            "a token that is no longer the stored one marks nothing"
        );
        mark_needs_pairing(&dir, "box", "tok").unwrap();
        assert!(load(&dir).unwrap().get("box").unwrap().needs_pairing);
        assert!(forget(&dir, "box").unwrap());
        assert!(!forget(&dir, "box").unwrap());
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            2,
            "the hosts and the lock, no temp file"
        );
    }

    #[test]
    fn a_newer_file_is_refused() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join(HOSTS_FILE),
            r#"{"version": 2, "hosts": []}"#,
        )
        .unwrap();
        assert!(load(root.path()).unwrap_err().contains("newer"));
    }
}
