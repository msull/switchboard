//! A pipeline command's confinement. The builders (the seatbelt profile,
//! the wrapped argv, the header line) are pure and compiled everywhere,
//! so their tests run on every platform; only `wrap`, which picks the
//! mechanism, is behind `cfg`. On macOS a command runs under
//! `sandbox-exec` with writes allowed only to the paths it was given and
//! a small base set; elsewhere it runs unconfined and its header says so.
//! `spikes/11-gate-sandbox/README.md` has the measurements behind the
//! profile.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::git::{Confine, Network};

/// The shell that runs a wrapped command and, when it fails, appends the
/// kernel's deny lines for it to stderr. Its arguments are the check's
/// log token and then the `sandbox-exec` argv, so nothing a pipeline
/// names is ever part of the script's text.
///
/// The kernel's line can reach the log after the command has exited, so
/// a query that finds nothing is tried once more a second later.
///
/// A status over 128 whose low part is a signal this system knows is
/// taken for a death by that signal: after the query, the shell
/// re-raises it on itself, so the runner reads the same status it would
/// have read for the bare command. A command that exits 137 by hand is
/// read as killed; one that exits 255 keeps its code.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) const SCRIPT: &str = r#"tok=$1; shift
start=$(date '+%Y-%m-%d %H:%M:%S')
"$@"
s=$?
if [ "$s" -eq 0 ]; then exit 0; fi
denied() {
  /usr/bin/log show --start "$start" --style compact --predicate "eventMessage CONTAINS \"$tok\"" 2>/dev/null | grep 'deny(' | sed 's/.*Sandbox: /dispatch: sandbox: /'
}
lines=$(denied)
if [ -z "$lines" ]; then sleep 1; lines=$(denied); fi
if [ -n "$lines" ]; then printf '%s\n' "$lines" >&2; fi
if [ "$s" -gt 128 ] && kill -l "$((s - 128))" >/dev/null 2>&1; then
  sig=$((s - 128))
  trap - "$sig" 2>/dev/null
  kill -"$sig" $$
fi
exit "$s"
"#;

/// Where `sandbox-exec` lives on every macOS since 10.5.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// The SBPL profile for `n_paths` writable paths, passed as parameters
/// `W0`..`Wn`, plus `TMP` and the log token `TOKEN`. Paths never appear
/// in the text, so no quoting can break out of it.
///
/// Writes to the devices a shell and a pty need stay allowed: a test
/// runner that opens a pty (tmux, Python's `pty`) fails without
/// `/dev/ptmx` and `/dev/ttys*`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn seatbelt_profile(n_paths: usize, network: Network) -> String {
    let mut text = String::from(
        "(version 1)\n(allow default)\n(deny file-write* (with message (param \"TOKEN\")))\n(allow file-write*",
    );
    for i in 0..n_paths {
        let _ = write!(text, " (subpath (param \"W{i}\"))");
    }
    text.push_str(
        " (subpath (param \"TMP\")) (literal \"/dev/null\") (literal \"/dev/tty\") \
         (literal \"/dev/dtracehelper\") (literal \"/dev/ptmx\") \
         (regex #\"^/dev/ttys[0-9]+$\") (subpath \"/dev/fd\"))\n",
    );
    if network == Network::Deny {
        // Loopback stays open: a test that starts a local service (a
        // local database) talks to it over 127.0.0.1.
        text.push_str(
            "(deny network-outbound (remote ip))\n(allow network-outbound (remote ip \"localhost:*\"))\n",
        );
    }
    text
}

/// The full argv: the reporting shell, `sandbox-exec` with the profile
/// and every path as a `-D` parameter, then `argv` after `--`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn seatbelt_argv(
    argv: &[String],
    writable: &[PathBuf],
    tmp: &Path,
    network: Network,
    token: &str,
) -> Vec<String> {
    let mut out: Vec<String> = vec![
        "sh".into(),
        "-c".into(),
        SCRIPT.into(),
        "sh".into(),
        token.into(),
        SANDBOX_EXEC.into(),
        "-p".into(),
        seatbelt_profile(writable.len(), network),
    ];
    for (i, p) in writable.iter().enumerate() {
        out.push("-D".into());
        out.push(format!("W{i}={}", p.display()));
    }
    out.push("-D".into());
    out.push(format!("TMP={}", tmp.display()));
    out.push("-D".into());
    out.push(format!("TOKEN={token}"));
    out.push("--".into());
    out.extend(argv.iter().cloned());
    out
}

/// The first line of a confined command's output: what it may write and
/// whether it has the network.
pub(crate) fn header(c: &Confine) -> String {
    let paths: Vec<String> = c.writable.iter().map(|p| p.display().to_string()).collect();
    let network = match c.network {
        Network::Allow => "allow",
        Network::Deny => "deny",
    };
    format!(
        "dispatch: confined; writable {}; network {network}",
        paths.join(", ")
    )
}

/// The argv to spawn for `argv` under `c`, and the header line to write
/// before its output.
#[cfg(target_os = "macos")]
pub(crate) fn wrap(argv: &[String], c: &Confine) -> Result<(Vec<String>, String)> {
    if !Path::new(SANDBOX_EXEC).exists() {
        anyhow::bail!("confinement is on and {SANDBOX_EXEC} is missing");
    }
    let mut writable: Vec<PathBuf> = Vec::new();
    for p in &c.writable {
        let real = resolve(p);
        if let Some(git_dir) = worktree_git_dir(&real) {
            push_once(&mut writable, git_dir);
        }
        push_once(&mut writable, real);
    }
    let tmp = resolve(&std::env::temp_dir());
    let shown = Confine {
        writable: writable.clone(),
        network: c.network,
    };
    Ok((
        seatbelt_argv(argv, &writable, &tmp, c.network, &token()),
        header(&shown),
    ))
}

/// The argv to spawn for `argv` under `c`, and the header line to write
/// before its output. The signature matches the macOS arm, which can
/// fail to resolve a path, so the callers are the same on every platform.
#[cfg(not(target_os = "macos"))]
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn wrap(argv: &[String], _c: &Confine) -> Result<(Vec<String>, String)> {
    log::warn!("confinement is on but this platform has no sandbox; running unconfined");
    Ok((
        argv.to_vec(),
        "dispatch: unconfined (no sandbox on this platform)".to_owned(),
    ))
}

/// A token unique to one wrapped command, put on its deny lines so the
/// log query finds this command's lines and no other sandbox's.
#[cfg(target_os = "macos")]
fn token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "dispatch-{:x}-{:x}-{nanos:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// `p` with symlinks resolved, as seatbelt matches resolved paths
/// (`/tmp` is `/private/tmp`). A path that does not exist yet keeps its
/// missing tail on top of its longest existing ancestor, resolved.
#[cfg(target_os = "macos")]
fn resolve(p: &Path) -> PathBuf {
    if let Ok(real) = p.canonicalize() {
        return real;
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) if parent != p => resolve(parent).join(name),
        _ => p.to_path_buf(),
    }
}

/// A linked worktree's own git directory (`<clone>/.git/worktrees/<name>`),
/// which holds its index and `HEAD`. Never the clone's whole `.git`, whose
/// refs and objects every ticket shares.
#[cfg(target_os = "macos")]
fn worktree_git_dir(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--absolute-git-dir"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let git_dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let parent = git_dir.parent()?;
    (parent.file_name()? == "worktrees" && parent.parent()?.file_name()? == ".git")
        .then(|| resolve(&git_dir))
}

#[cfg(target_os = "macos")]
fn push_once(paths: &mut Vec<PathBuf>, p: PathBuf) {
    if !paths.contains(&p) {
        paths.push(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|&i| i.to_owned()).collect()
    }

    #[test]
    fn the_profile_has_one_parameter_per_path_and_denies_the_network_only_when_asked() {
        let allow = seatbelt_profile(3, Network::Allow);
        for i in 0..3 {
            assert!(allow.contains(&format!("(subpath (param \"W{i}\"))")));
        }
        assert!(!allow.contains("W3"));
        assert!(!allow.contains("network-outbound"));
        let deny = seatbelt_profile(1, Network::Deny);
        assert!(deny.contains("(deny network-outbound (remote ip))"));
        assert!(deny.contains("localhost:*"));
    }

    #[test]
    fn paths_reach_sandbox_exec_only_as_parameters_and_the_command_follows_the_last_dashes() {
        let argv = s(&["sh", "-c", "touch ok"]);
        let paths = [PathBuf::from("/w/it's \"odd\""), PathBuf::from("/c")];
        let out = seatbelt_argv(&argv, &paths, Path::new("/t"), Network::Allow, "tok1");
        let profile_at = out.iter().position(|a| a == "-p").unwrap() + 1;
        assert!(!out[profile_at].contains("/w/"));
        assert!(!out[profile_at].contains("/c"));
        assert!(out.contains(&"W0=/w/it's \"odd\"".to_owned()));
        assert!(out.contains(&"W1=/c".to_owned()));
        assert!(out.contains(&"TMP=/t".to_owned()));
        assert!(out.contains(&"TOKEN=tok1".to_owned()));
        assert_eq!(&out[..5], &s(&["sh", "-c", SCRIPT, "sh", "tok1"])[..]);
        assert_eq!(out[5], SANDBOX_EXEC);
        let dashes = out.iter().rposition(|a| a == "--").unwrap();
        assert_eq!(&out[dashes + 1..], &argv[..]);
        assert!(!SCRIPT.contains("/w/"));
    }

    #[test]
    fn the_header_names_the_paths_and_the_network() {
        let c = Confine {
            writable: vec![PathBuf::from("/w"), PathBuf::from("/a")],
            network: Network::Deny,
        };
        assert_eq!(
            header(&c),
            "dispatch: confined; writable /w, /a; network deny"
        );
    }
}
