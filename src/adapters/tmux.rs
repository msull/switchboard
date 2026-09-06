//! tmux process host on a private socket (`-L switchboard`) with the app's
//! own config file, so it never sees the user's sessions or `~/.tmux.conf`.
//! Every method is one `std::process::Command` running the tmux client;
//! the server starts implicitly on the first `new-session`. The command
//! mapping and the measured quirks come from spike 02.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::ports::host::{HostId, HostInfo, HostStatus, Liveness, ProcessHost, SpawnSpec};

/// Oldest tmux that has `new-session -e` and `pane_dead_status`.
const MIN_VERSION: (u32, u32) = (3, 2);

/// One line per pane, tab separated, in the order `parse_status` expects.
const STATUS_FORMAT: &str = "#{session_name}\t#{pane_dead}\t#{pane_dead_status}\t#{pane_pid}\t\
                             #{pane_current_command}\t#{pane_current_path}\t#{window_activity}\t\
                             #{pane_title}";

/// The tmux config the private server runs with. `remain-on-exit` keeps
/// dead panes around so exit codes are observable; `set-titles` passes
/// OSC titles (Claude Code sets them) through to `pane_title`.
const DEFAULT_CONFIG: &str = "\
set -g remain-on-exit on
set -g history-limit 50000
set -g default-terminal \"tmux-256color\"
set -g mouse on
set -g status off
set -g window-size manual
set -g allow-passthrough on
set -g escape-time 10
set -g focus-events on
set -g set-titles on
set -g set-titles-string \"#{pane_title}\"
";

/// A handle to one private tmux server, addressed by socket name.
#[derive(Debug, Clone)]
pub struct TmuxHost {
    socket: String,
    config: PathBuf,
    bin: PathBuf,
}

impl TmuxHost {
    /// `config_path` is passed as `-f`; `None` means `/dev/null`, which
    /// keeps the user's `~/.tmux.conf` out. The binary is `SWITCHBOARD_TMUX`
    /// when set (a bundled copy, say), otherwise `tmux` from `PATH`.
    #[must_use]
    pub fn new(socket_name: &str, config_path: Option<PathBuf>) -> Self {
        Self {
            socket: socket_name.to_owned(),
            config: config_path.unwrap_or_else(|| PathBuf::from("/dev/null")),
            bin: std::env::var_os("SWITCHBOARD_TMUX").map_or_else(locate_tmux, PathBuf::from),
        }
    }

    /// The socket the app's server lives on.
    #[must_use]
    pub fn default_socket() -> &'static str {
        "switchboard"
    }

    /// Write the app's tmux config to `path` (parent directories created).
    pub fn write_default_config(path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, DEFAULT_CONFIG)
    }

    /// Stop `pipe-pane` on the session and start it again on `new_path`.
    /// A bare rename of the old file is not enough: the old `cat` keeps its
    /// descriptor, so the pipe has to be restarted (design: scrollback
    /// rotation).
    pub fn rotate_scrollback(&self, id: &HostId, new_path: &Path) -> io::Result<()> {
        // `pipe-pane` with no command turns the pipe off.
        self.run(&["pipe-pane", "-t", &pane_target(id)])?;
        self.pipe_to(id, new_path)
    }

    /// Kill the whole server on this socket. For tests; the app never does
    /// this because the server is what keeps sessions alive.
    pub fn kill_server(&self) -> io::Result<()> {
        match self.run(&["kill-server"]) {
            Ok(_) => Ok(()),
            Err(e) if is_no_server(&e) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// A `Command` for this server: `tmux -L <socket> -f <config> ...`.
    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("-L").arg(&self.socket).arg("-f").arg(&self.config);
        // The server inherits this process's environment on first start,
        // and every pane inherits the server's. An app launched from the
        // Dock gets launchd's minimal PATH, so add the usual tool dirs.
        cmd.env("PATH", augmented_path());
        // Without a UTF-8 locale tmux replaces the tab separators in
        // `-F` output with `_` (and a server started that way treats pane
        // output as ASCII). Dock-launched apps have no LANG at all.
        if !has_utf8_locale() {
            cmd.env("LANG", "en_US.UTF-8");
        }
        // When Switchboard itself was launched from inside a Claude Code
        // session, the inherited CLAUDE* variables make child agents skip
        // transcript writes, which breaks resume. Strip them.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("CLAUDE") {
                cmd.env_remove(&key);
            }
        }
        cmd
    }

    /// Run one tmux command and return its stdout, or its stderr as the
    /// error message on a non-zero exit.
    fn run(&self, args: &[&str]) -> io::Result<String> {
        let out = self.command().args(args).output()?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&out.stderr).trim_end().to_owned(),
            ))
        }
    }

    fn pipe_to(&self, id: &HostId, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let shell = format!("cat >> {}", shell_quote(&path.to_string_lossy()));
        // `-o` only starts the pipe when none is running, so re-asserting
        // it at reattach does not toggle it off.
        self.run(&["pipe-pane", "-o", "-t", &pane_target(id), &shell])?;
        Ok(())
    }
}

impl ProcessHost for TmuxHost {
    fn probe(&self) -> Result<HostInfo, String> {
        let hint = format!(
            "Switchboard needs tmux {}.{} or newer (brew install tmux)",
            MIN_VERSION.0, MIN_VERSION.1
        );
        let out = Command::new(&self.bin)
            .arg("-V")
            .output()
            .map_err(|e| format!("tmux not found ({}): {e}. {hint}", self.bin.display()))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("`tmux -V` failed: {}. {hint}", stderr.trim()));
        }
        let banner = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let version = parse_version(&banner)
            .ok_or_else(|| format!("unrecognised `tmux -V` output {banner:?}. {hint}"))?;
        if version < MIN_VERSION {
            return Err(format!("found {banner}, which is too old. {hint}"));
        }
        Ok(HostInfo {
            description: banner,
            persistent: true,
        })
    }

    fn list(&self) -> io::Result<Vec<HostStatus>> {
        let out = match self.run(&["list-panes", "-a", "-F", STATUS_FORMAT]) {
            Ok(out) => out,
            // No server means no sessions, which is the normal cold start.
            Err(e) if is_no_server(&e) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut statuses: Vec<HostStatus> = Vec::new();
        for line in out.lines() {
            let Some(status) = parse_status(line) else {
                log::warn!("unparseable list-panes line: {line:?}");
                continue;
            };
            // A session's first pane stands for the session; the app
            // never opens more windows, so extras are the user's doing.
            if !statuses.iter().any(|s| s.id == status.id) {
                statuses.push(status);
            }
        }
        Ok(statuses)
    }

    fn spawn(&self, spec: &SpawnSpec) -> io::Result<()> {
        if self
            .run(&["has-session", "-t", &session_target(&spec.id)])
            .is_ok()
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("tmux session {:?} already exists", spec.id.0),
            ));
        }
        let cwd = spec.cwd.to_string_lossy().into_owned();
        let mut args: Vec<String> = [
            "new-session",
            "-d",
            "-s",
            &spec.id.0,
            "-c",
            &cwd,
            "-x",
            "200",
            "-y",
            "50",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        for (k, v) in &spec.env {
            args.push("-e".into());
            args.push(format!("{k}={v}"));
        }
        if let Some(argv) = &spec.command {
            // tmux takes one shell-command string; quoting each element
            // keeps the argv boundaries through its `sh -c`.
            let quoted: Vec<String> = argv.iter().map(|a| shell_quote(a)).collect();
            args.push(quoted.join(" "));
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&args)?;
        // A readable default title until the program sets its own.
        self.run(&[
            "select-pane",
            "-T",
            &spec.id.0,
            "-t",
            &pane_target(&spec.id),
        ])?;
        if let Some(path) = &spec.scrollback {
            self.pipe_to(&spec.id, path)?;
        }
        Ok(())
    }

    fn status(&self, id: &HostId) -> io::Result<HostStatus> {
        let missing = || HostStatus {
            id: id.clone(),
            liveness: Liveness::Missing,
            cwd: None,
            last_activity: None,
            title: None,
        };
        match self.run(&["list-panes", "-t", &session_target(id), "-F", STATUS_FORMAT]) {
            Ok(out) => Ok(out.lines().find_map(parse_status).unwrap_or_else(missing)),
            Err(e) if is_no_server(&e) || is_not_found(&e) => Ok(missing()),
            Err(e) => Err(e),
        }
    }

    fn snapshot(&self, id: &HostId, lines: Option<usize>) -> io::Result<String> {
        // `-S -N` starts N lines above the visible screen; `-S -` is the
        // top of history.
        let start = lines.map_or_else(|| "-".to_owned(), |n| format!("-{n}"));
        let out = self.run(&["capture-pane", "-p", "-t", &pane_target(id), "-S", &start])?;
        // The pane is 50 rows, so a short session ends in blank lines.
        let kept: Vec<&str> = out
            .lines()
            .rev()
            .skip_while(|l| l.trim().is_empty())
            .collect();
        Ok(kept.into_iter().rev().collect::<Vec<_>>().join("\n"))
    }

    fn write(&self, id: &HostId, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let target = pane_target(id);
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut args = vec!["send-keys", "-t", &target, "-H"];
        args.extend(hex.iter().map(String::as_str));
        self.run(&args)?;
        Ok(())
    }

    fn kill(&self, id: &HostId) -> io::Result<()> {
        self.run(&["kill-session", "-t", &session_target(id)])?;
        Ok(())
    }

    fn attach_command(&self, id: &HostId) -> Vec<String> {
        vec![
            self.bin.to_string_lossy().into_owned(),
            "-L".into(),
            self.socket.clone(),
            "-f".into(),
            self.config.to_string_lossy().into_owned(),
            "attach-session".into(),
            "-t".into(),
            session_target(id),
        ]
    }
}

/// Whether the locale variables tmux consults name a UTF-8 codeset.
fn has_utf8_locale() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(std::env::var_os)
        .any(|v| v.to_string_lossy().to_ascii_uppercase().contains("UTF-8"))
}

/// Directories a Dock-launched app's PATH lacks but a login shell has.
fn extra_bin_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/bin"),
    ]
}

/// `tmux` from `PATH`, else from the usual install dirs, as an absolute
/// path so the same binary is handed to Ghostty. Falls back to the bare
/// name, which lets the probe report "not found".
fn locate_tmux() -> PathBuf {
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    path_dirs
        .into_iter()
        .chain(extra_bin_dirs())
        .map(|d| d.join("tmux"))
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("tmux"))
}

/// The current PATH with any missing [`extra_bin_dirs`] appended.
fn augmented_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for d in extra_bin_dirs() {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// `=name`: exact session match, so `foo` never resolves to `foobar`.
fn session_target(id: &HostId) -> String {
    format!("={}", id.0)
}

/// Pane-addressed commands (`send-keys`, `capture-pane`, `pipe-pane`,
/// `select-pane`) reject a bare `=name`; `=name:` means that session's
/// current window and its active pane.
fn pane_target(id: &HostId) -> String {
    format!("={}:", id.0)
}

fn is_no_server(e: &io::Error) -> bool {
    let msg = e.to_string();
    msg.contains("no server running") || msg.contains("error connecting")
}

fn is_not_found(e: &io::Error) -> bool {
    e.to_string().contains("can't find")
}

/// `tmux 3.7c` -> `(3, 7)`; also accepts `tmux next-3.8` and `3.2a`.
fn parse_version(banner: &str) -> Option<(u32, u32)> {
    let rest = banner.trim().strip_prefix("tmux").unwrap_or(banner).trim();
    let rest = rest.strip_prefix("next-").unwrap_or(rest);
    let mut parts = rest.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: String = parts
        .next()?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    Some((major, minor.parse().ok()?))
}

/// One `STATUS_FORMAT` line into a status; `None` for a malformed line.
fn parse_status(line: &str) -> Option<HostStatus> {
    let mut f = line.split('\t');
    let name = f.next()?;
    let dead = f.next()? == "1";
    let dead_status = f.next()?;
    let pid = f.next()?;
    let command = f.next()?;
    let path = f.next()?;
    let activity = f.next()?;
    let title = f.next().unwrap_or("");
    let liveness = if dead {
        Liveness::Exited {
            code: dead_status.parse().ok(),
        }
    } else {
        Liveness::Running {
            pid: pid.parse().ok()?,
            command: command.to_owned(),
        }
    };
    Some(HostStatus {
        id: HostId(name.to_owned()),
        liveness,
        cwd: (!path.is_empty()).then(|| PathBuf::from(path)),
        last_activity: activity
            .parse::<u64>()
            .ok()
            .filter(|&s| s > 0)
            .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
        title: (!title.is_empty()).then(|| title.to_owned()),
    })
}

/// Single-quote `s` for `sh` unless it is made only of safe characters.
/// Leaving plain words bare matters: tmux derives `pane_current_command`
/// of a dead pane from the command string, and `'sh'` reads badly.
fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("tmux 3.7c"), Some((3, 7)));
        assert_eq!(parse_version("tmux 3.2a"), Some((3, 2)));
        assert_eq!(parse_version("tmux next-3.8"), Some((3, 8)));
        assert_eq!(parse_version("tmux 2.9"), Some((2, 9)));
        assert_eq!(parse_version("nonsense"), None);
        assert!((3, 2) >= MIN_VERSION);
        assert!((2, 9) < MIN_VERSION);
    }

    #[test]
    fn parses_status_lines() {
        let running = parse_status("s1\t0\t\t123\tzsh\t/tmp\t1788678297\tmy title").unwrap();
        assert_eq!(running.id, HostId("s1".into()));
        assert_eq!(
            running.liveness,
            Liveness::Running {
                pid: 123,
                command: "zsh".into()
            }
        );
        assert_eq!(running.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(running.title.as_deref(), Some("my title"));
        assert_eq!(
            running.last_activity,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_788_678_297))
        );

        let dead = parse_status("s2\t1\t3\t9\tsh\t\t0\t").unwrap();
        assert_eq!(dead.liveness, Liveness::Exited { code: Some(3) });
        assert_eq!(dead.cwd, None);
        assert_eq!(dead.last_activity, None);
        assert_eq!(dead.title, None);
        assert!(parse_status("garbage").is_none());
    }

    #[test]
    fn quotes_for_sh() {
        assert_eq!(shell_quote("sh"), "sh");
        assert_eq!(shell_quote("-c"), "-c");
        assert_eq!(shell_quote("exit 3"), "'exit 3'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn attach_command_uses_socket_and_config() {
        let host = TmuxHost::new("switchboard", Some(PathBuf::from("/x/tmux.conf")));
        let argv = host.attach_command(&HostId("abc".into()));
        assert_eq!(
            &argv[1..],
            [
                "-L",
                "switchboard",
                "-f",
                "/x/tmux.conf",
                "attach-session",
                "-t",
                "=abc"
            ]
        );
        assert_eq!(TmuxHost::default_socket(), "switchboard");
    }

    #[test]
    fn command_carries_tool_dirs_and_a_utf8_locale() {
        let host = TmuxHost::new("switchboard-test-env", None);
        let cmd = host.command();
        let envs: Vec<_> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_string_lossy().into_owned(), v?.to_owned())))
            .collect();
        let path = envs.iter().find(|(k, _)| k == "PATH").expect("PATH set");
        assert!(path.1.to_string_lossy().contains("/opt/homebrew/bin"));
        let lang = envs.iter().find(|(k, _)| k == "LANG");
        assert_eq!(lang.is_some(), !has_utf8_locale());
    }

    #[test]
    fn probe_fails_without_binary() {
        let host = TmuxHost {
            socket: "switchboard-test-none".into(),
            config: PathBuf::from("/dev/null"),
            bin: PathBuf::from("/nonexistent/tmux"),
        };
        let err = host.probe().unwrap_err();
        assert!(err.contains("brew install tmux"), "{err}");
    }

    // ---- integration tests: each on its own `switchboard-test-*` socket ----

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A host on a fresh test socket whose server dies with the guard,
    /// even when the test panics. `_dir` keeps the temp config alive.
    struct Server {
        host: TmuxHost,
        _dir: tempfile::TempDir,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.host.kill_server();
        }
    }

    /// `None` when tmux is unusable here, so CI without tmux stays green.
    fn server() -> Option<Server> {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let socket = format!("switchboard-test-{}-{n}", std::process::id());
        let dir = tempfile::tempdir().expect("temp dir");
        let config = dir.path().join("tmux.conf");
        TmuxHost::write_default_config(&config).expect("write config");
        let host = TmuxHost::new(&socket, Some(config));
        if let Err(e) = host.probe() {
            eprintln!("skipping tmux integration test: {e}");
            return None;
        }
        Some(Server { host, _dir: dir })
    }

    fn poll(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn shell_spec(id: &str, dir: &Path, scrollback: Option<PathBuf>) -> SpawnSpec {
        SpawnSpec {
            id: HostId(id.into()),
            cwd: dir.to_path_buf(),
            command: None,
            env: vec![("SWITCHBOARD_TEST".into(), "hello-from-tmux".into())],
            scrollback,
        }
    }

    /// Type a line the way the app will: raw bytes plus a carriage return.
    fn type_line(host: &TmuxHost, id: &HostId, line: &str) {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\r');
        host.write(id, &bytes).expect("send-keys");
    }

    fn file_contains(path: &Path, needle: &str) -> bool {
        std::fs::read(path).is_ok_and(|b| String::from_utf8_lossy(&b).contains(needle))
    }

    #[test]
    fn probe_reports_version() {
        let Some(s) = server() else { return };
        let info = s.host.probe().unwrap();
        assert!(
            info.description.starts_with("tmux "),
            "{}",
            info.description
        );
        assert!(info.persistent);
    }

    #[test]
    fn shell_session_with_env_cwd_and_scrollback() {
        let Some(s) = server() else { return };
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().canonicalize().unwrap();
        let log = dir.path().join("logs/a.log");
        let id = HostId("shell".into());
        s.host
            .spawn(&shell_spec("shell", &cwd, Some(log.clone())))
            .unwrap();

        type_line(&s.host, &id, "printf \"$SWITCHBOARD_TEST\\n\"");
        poll("env var in snapshot", || {
            s.host
                .snapshot(&id, Some(50))
                .unwrap()
                .contains("hello-from-tmux")
        });
        let full = s.host.snapshot(&id, None).unwrap();
        assert!(full.contains("hello-from-tmux"));
        assert!(!full.ends_with('\n'), "trailing blank lines trimmed");

        let status = s.host.status(&id).unwrap();
        assert!(
            matches!(status.liveness, Liveness::Running { pid, .. } if pid > 0),
            "{status:?}"
        );
        assert_eq!(status.cwd.as_deref(), Some(cwd.as_path()));
        assert!(status.last_activity.is_some());
        assert!(status.title.is_some());

        // pipe-pane streamed the raw bytes to disk.
        poll("scrollback file", || file_contains(&log, "hello-from-tmux"));

        // Rotation: later output lands in the new file only.
        let log2 = dir.path().join("logs/b.log");
        s.host.rotate_scrollback(&id, &log2).unwrap();
        let old_len = std::fs::metadata(&log).unwrap().len();
        type_line(&s.host, &id, "printf ROTATED-%s\\\\n MARK");
        poll("rotated scrollback", || {
            file_contains(&log2, "ROTATED-MARK")
        });
        assert_eq!(
            std::fs::metadata(&log).unwrap().len(),
            old_len,
            "old file stopped growing"
        );

        // Spawning the same id again is refused.
        let err = s.host.spawn(&shell_spec("shell", &cwd, None)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);

        // A second instance on the same socket sees the session (reattach).
        let other = TmuxHost::new(&s.host.socket, Some(s.host.config.clone()));
        let listed = other.list().unwrap();
        assert!(listed.iter().any(|h| h.id == id), "{listed:?}");
        assert!(
            other
                .snapshot(&id, None)
                .unwrap()
                .contains("hello-from-tmux")
        );

        s.host.kill(&id).unwrap();
        assert_eq!(s.host.status(&id).unwrap().liveness, Liveness::Missing);
        assert!(!s.host.list().unwrap().iter().any(|h| h.id == id));
    }

    #[test]
    fn command_exit_code_is_reported() {
        let Some(s) = server() else { return };
        let id = HostId("job".into());
        let spec = SpawnSpec {
            id: id.clone(),
            cwd: std::env::temp_dir(),
            command: Some(vec!["sh".into(), "-c".into(), "exit 3".into()]),
            env: Vec::new(),
            scrollback: None,
        };
        s.host.spawn(&spec).unwrap();
        poll("exit status", || {
            s.host.status(&id).unwrap().liveness == Liveness::Exited { code: Some(3) }
        });
        let listed = s.host.list().unwrap();
        let job = listed.iter().find(|h| h.id == id).expect("listed");
        assert_eq!(job.liveness, Liveness::Exited { code: Some(3) });
        assert_eq!(job.cwd, None);
    }

    #[test]
    fn no_server_is_an_empty_list() {
        let Some(s) = server() else { return };
        // Same socket, nothing spawned on it, `-f /dev/null`.
        let host = TmuxHost::new(&s.host.socket, None);
        assert_eq!(host.list().unwrap(), Vec::new());
        assert_eq!(
            host.status(&HostId("x".into())).unwrap().liveness,
            Liveness::Missing
        );
        host.kill_server().unwrap();
    }
}
