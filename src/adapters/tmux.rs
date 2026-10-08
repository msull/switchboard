//! tmux process host on a private socket (`-L switchboard`) with the app's
//! own config file, so it never sees the user's sessions or `~/.tmux.conf`.
//! Every method is one `std::process::Command` running the tmux client.
//! The server process keeps the argv of the client that started it for
//! as long as it runs, so a spawn starts it with a bare `start-server`
//! and sends `new-session` (with its `-e` values and pane command) on
//! the client's stdin through `source-file -` (spike 18). The command
//! mapping and the measured quirks come from spike 02.

use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use crate::ports::host::{HostId, HostInfo, HostStatus, Liveness, ProcessHost, SpawnSpec};

/// Oldest tmux that has `new-session -e`, `pane_dead_status` and
/// `source-file -`.
const MIN_VERSION: (u32, u32) = (3, 2);

/// The tmux client's argv for every spawn. The `";"` is its own element
/// because no shell is involved: that is how tmux separates commands.
/// `start-server` does nothing when a server runs already, and
/// `source-file` cannot start one on its own. Everything taken from a
/// `SpawnSpec` goes on stdin (`spawn_script`), never here.
const SPAWN_ARGS: [&str; 4] = ["start-server", ";", "source-file", "-"];

/// One line per pane, tab separated, in the order `parse_status` expects.
const STATUS_FORMAT: &str = "#{session_name}\t#{pane_dead}\t#{pane_dead_status}\t#{pane_pid}\t\
                             #{pane_current_command}\t#{pane_current_path}\t#{window_activity}\t\
                             #{pane_title}";

/// The tmux config the private server runs with. `remain-on-exit` keeps
/// dead panes around so exit codes are observable; `set-titles` passes
/// OSC titles (Claude Code sets them) through to `pane_title`;
/// `window-size latest` lets a window follow whichever client attached
/// last (the embedded terminal, a Ghostty window), while a detached
/// window keeps the size it was created with.
const DEFAULT_CONFIG: &str = "\
set -g remain-on-exit on
set -g history-limit 50000
set -g default-terminal \"tmux-256color\"
set -g mouse on
set -g status off
set -g window-size latest
set -g allow-passthrough on
set -g escape-time 10
set -g focus-events on
set -g set-titles on
set -g set-titles-string \"#{pane_title}\"
";

/// Gap between typed text and the Enter that submits it.
const ENTER_DELAY: std::time::Duration = std::time::Duration::from_millis(150);

/// Bytes per `send-keys` call. Each byte is one hex argument, and tmux
/// refuses a command past a few thousand arguments ("command too
/// long"), so a long message goes over in pieces.
const SEND_CHUNK: usize = 512;

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

    /// Start and kill a throwaway session, to learn whether this tmux
    /// can host one at all. A version check is not enough: Ubuntu's
    /// tmux 3.4 dies at window spawn under `window-size manual` on
    /// GitHub's runners (a plain container is fine), so the tests call
    /// this to skip cleanly there.
    pub fn smoke_test(&self) -> io::Result<()> {
        // The same path `spawn` takes, so a tmux that cannot read
        // commands from stdin is skipped here rather than failing every
        // spawn test.
        let smoke = SpawnSpec {
            id: HostId("smoke".into()),
            cwd: PathBuf::from("/"),
            command: None,
            env: Vec::new(),
            scrollback: None,
        };
        let started = self.run_with_stdin(&SPAWN_ARGS, spawn_script(&smoke).as_bytes());
        let _ = self.run(&["kill-session", "-t", "=smoke"]);
        // Killing the last session makes the server exit on its own
        // time; a `new-session` that reaches it first is answered
        // "server exited unexpectedly". Wait until it is gone.
        for _ in 0..40 {
            match self.run(&["list-sessions"]) {
                Err(e) if is_no_server(&e) => break,
                _ => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        started.map(|_| ())
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

    /// Re-apply the options a running server may have started without.
    /// The server outlives the app, so a config change only reaches an
    /// old server this way. No server is fine: the next spawn starts one
    /// from the config file.
    pub fn apply_options(&self) -> io::Result<()> {
        match self.run(&["set-option", "-g", "window-size", "latest"]) {
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

    /// `run` with `input` on the command's stdin, for text that must not
    /// go on a command line: there is no argument limit on stdin, and a
    /// message never shows in a process listing.
    fn run_with_stdin(&self, args: &[&str], input: &[u8]) -> io::Result<String> {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Dropping the handle closes the pipe, which is the end of input
        // tmux waits for. A failed write still waits for the child, so it
        // is reaped, and tmux's own complaint wins over the broken pipe it
        // caused.
        let written = child
            .stdin
            .take()
            .map_or(Ok(()), |mut stdin| stdin.write_all(input));
        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&out.stderr).trim_end().to_owned(),
            ));
        }
        written?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Text with a line break goes as one paste, so a TUI reads its line
    /// breaks as part of the message rather than as Enter. `-p` brackets
    /// it only when the pane enabled bracketed paste, so a plain process
    /// never sees the markers; `-r` keeps LF, which tmux would otherwise
    /// turn into CR (an Enter per line); `-d` drops the buffer so the
    /// message does not linger in the server's buffer list.
    fn paste(&self, id: &HostId, text: &str) -> io::Result<()> {
        let buffer = format!("sb-send-{}", id.0);
        self.run_with_stdin(
            &["load-buffer", "-b", &buffer, "-"],
            paste_body(text).as_bytes(),
        )?;
        let pasted = self.run(&[
            "paste-buffer",
            "-p",
            "-r",
            "-d",
            "-b",
            &buffer,
            "-t",
            &pane_target(id),
        ]);
        if pasted.is_err() {
            let _ = self.run(&["delete-buffer", "-b", &buffer]);
        }
        pasted.map(drop)
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
        // The environment values (secrets among them) and the pane
        // command go on stdin, so no process listing ever shows them.
        self.run_with_stdin(&SPAWN_ARGS, spawn_script(spec).as_bytes())?;
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
        for chunk in bytes.chunks(SEND_CHUNK) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            let mut args = vec!["send-keys", "-t", &target, "-H"];
            args.extend(hex.iter().map(String::as_str));
            self.run(&args)?;
        }
        Ok(())
    }

    fn write_line(&self, id: &HostId, text: &str) -> io::Result<()> {
        // Enter must follow the paste's closing marker, or a TUI reads it
        // as part of the paste; the thread below starts only once the text
        // has gone out, which orders it after.
        if text.contains(['\n', '\r']) {
            self.paste(id, text)?;
        } else {
            self.write(id, text.as_bytes())?;
        }
        // The handle is a socket name and two paths, so the thread gets
        // its own copy and the UI thread never waits on the pause.
        let host = self.clone();
        let id = id.clone();
        std::thread::spawn(move || {
            std::thread::sleep(ENTER_DELAY);
            if let Err(e) = host.write(&id, b"\r") {
                log::warn!("enter after input to {} failed: {e}", id.0);
            }
        });
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
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        super::home_dir().join(".local/bin"),
    ]
}

/// `tmux` from `PATH`, else from the usual install dirs, as an absolute
/// path so the same binary is handed to Ghostty. Falls back to the bare
/// name, which lets the probe report "not found".
fn locate_tmux() -> PathBuf {
    let dirs = super::path_dirs().into_iter().chain(extra_bin_dirs());
    super::find_in(dirs, "tmux").unwrap_or_else(|| PathBuf::from("tmux"))
}

/// The current PATH with any missing [`extra_bin_dirs`] appended.
fn augmented_path() -> std::ffi::OsString {
    let mut dirs = super::path_dirs();
    for d in extra_bin_dirs() {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// The body of a paste: line breaks as LF and ESC removed. tmux passes
/// CR through unchanged, where a TUI may read it as Enter, and an ESC in
/// the text could start a sequence that closes the bracket early (tmux
/// 3.7 shows it as a literal `^[`, which is no better in a prompt).
fn paste_body(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\x1b', "")
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

/// A server that is not there, or is on its way out: after its last
/// session is killed tmux exits on its own time, and a command that
/// reaches it first is answered "server exited unexpectedly". Nothing
/// is hosted either way.
fn is_no_server(e: &io::Error) -> bool {
    let msg = e.to_string();
    msg.contains("no server running")
        || msg.contains("error connecting")
        || msg.contains("server exited unexpectedly")
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

/// Double-quote `s` for tmux's command parser, escaping every character
/// it would otherwise act on: `\`, `"`, `$` (environment expansion) and
/// `~` (home expansion at the start of a word). A line break is written
/// as `\n`, never literally, because tmux drops a line that starts with
/// `#` as a comment even inside quotes (single quotes too), which would
/// cut a Markdown heading out of a prompt or a value. `#{}`, `%` and `;`
/// mean nothing inside quotes. Unlike `shell_quote` it never leaves a word
/// bare, because `%`, `#` and `;` mean something to tmux that they do not
/// mean to `sh`.
fn tmux_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' | '"' | '$' | '~' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `new-session` line `spawn` writes to the tmux client's stdin,
/// ending in one newline. The pane command is `sh`-quoted per element,
/// which keeps the argv boundaries through tmux's `sh -c`, and the
/// joined string is then tmux-quoted as one argument.
fn spawn_script(spec: &SpawnSpec) -> String {
    let mut words = vec![
        "new-session".to_owned(),
        "-d".to_owned(),
        "-s".to_owned(),
        tmux_quote(&spec.id.0),
        "-c".to_owned(),
        tmux_quote(&spec.cwd.to_string_lossy()),
        "-x".to_owned(),
        "200".to_owned(),
        "-y".to_owned(),
        "50".to_owned(),
    ];
    for (k, v) in &spec.env {
        words.push("-e".to_owned());
        words.push(tmux_quote(&format!("{k}={v}")));
    }
    if let Some(argv) = &spec.command {
        let quoted: Vec<String> = argv.iter().map(|a| shell_quote(a)).collect();
        words.push(tmux_quote(&quoted.join(" ")));
    }
    let mut line = words.join(" ");
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
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
    fn quotes_for_tmux() {
        assert_eq!(tmux_quote(""), r#""""#);
        assert_eq!(tmux_quote("it's #{x} ; %"), r#""it's #{x} ; %""#);
        assert_eq!(tmux_quote(r#"~/a "q" $H \"#), r#""\~/a \"q\" \$H \\""#);
        assert_eq!(tmux_quote("a\n# b\r"), r#""a\n# b\r""#);
    }

    #[test]
    fn spawn_script_keeps_spec_off_the_argv() {
        let env_marker = format!("env-marker-{}", std::process::id());
        let cmd_marker = format!("cmd-marker-{}", std::process::id());
        let spec = SpawnSpec {
            id: HostId("a".into()),
            cwd: PathBuf::from("/tmp"),
            command: Some(vec!["echo".into(), cmd_marker.clone()]),
            env: vec![("SWITCHBOARD_TEST_MARKER".into(), env_marker.clone())],
            scrollback: None,
        };
        // `SPAWN_ARGS` is a fixed `const`, so the argv cannot carry the
        // spec; what matters is that the spec lands on stdin instead.
        let script = spawn_script(&spec);
        assert!(script.contains(&env_marker), "{script}");
        assert!(script.contains(&cmd_marker), "{script}");
        assert!(script.starts_with("new-session -d -s "), "{script}");
        assert!(
            script.ends_with('\n') && !script.ends_with("\n\n"),
            "{script}"
        );
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
        if let Err(e) = host.smoke_test() {
            eprintln!("skipping tmux integration test: cannot start a session here: {e}");
            let _ = host.kill_server();
            return None;
        }
        Some(Server { host, _dir: dir })
    }

    fn poll(what: &str, mut ready: impl FnMut() -> bool) {
        poll_on(None, what, &mut ready);
    }

    /// Like `poll`, and on timeout the panic carries what `server` had:
    /// every pane in the status format and the session list, so a
    /// failure on a machine that cannot be reached afterwards (CI) says
    /// which of missing, still running or dead-without-status it was.
    fn poll_on(server: Option<&Server>, what: &str, ready: &mut dyn FnMut() -> bool) {
        if let Err(seen) = try_poll_on(server, ready) {
            panic!("timed out waiting for {what}{seen}");
        }
    }

    /// The deadline loop under `poll_on` at its 20 s default, for a
    /// caller that writes its own failure message: on timeout, what
    /// `server` had. Generous because a cold CI runner needs the
    /// headroom to start a server and report a pane's exit.
    fn try_poll_on(server: Option<&Server>, ready: &mut dyn FnMut() -> bool) -> Result<(), String> {
        poll_for(server, Duration::from_secs(20), ready)
    }

    fn poll_for(
        server: Option<&Server>,
        timeout: Duration,
        ready: &mut dyn FnMut() -> bool,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        while !ready() {
            if Instant::now() >= deadline {
                return Err(server.map_or(String::new(), |s| {
                    let panes = s.host.run(&["list-panes", "-a", "-F", STATUS_FORMAT]);
                    let sessions = s.host.run(&["list-sessions"]);
                    format!(
                        "; panes: {panes:?}; sessions: {sessions:?}{}",
                        pane_process_states(s)
                    )
                }));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Ok(())
    }

    /// Each pane's process state from `/proc`: a `Z` means tmux never
    /// reaped its child, which tells a tmux defect from a slow exit.
    #[cfg(target_os = "linux")]
    fn pane_process_states(s: &Server) -> String {
        let pids = s
            .host
            .run(&["list-panes", "-a", "-F", "#{pane_pid}"])
            .unwrap_or_default();
        let states: Vec<String> = pids
            .split_whitespace()
            .map(|pid| {
                // The state follows the command name, which is in
                // parentheses and may itself hold spaces or parentheses.
                let state = std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        let (_, rest) = stat.rsplit_once(')')?;
                        rest.split_whitespace().next().map(str::to_owned)
                    })
                    .unwrap_or_else(|| "gone".into());
                format!("{pid}={state}")
            })
            .collect();
        format!("; pane processes: {}", states.join(" "))
    }

    #[cfg(not(target_os = "linux"))]
    fn pane_process_states(_: &Server) -> String {
        String::new()
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

    /// A window is created at a fixed size and then follows the size of
    /// the client that attaches, so an embedded terminal sees the whole
    /// pane. A control-mode client stands in for the widget here: it has
    /// no tty, so `refresh-client -C` tells tmux its size.
    #[test]
    fn window_follows_the_attached_client_size() {
        let Some(s) = server() else { return };
        let dir = tempfile::tempdir().unwrap();
        let id = HostId("sized".into());
        s.host
            .spawn(&shell_spec("sized", dir.path(), None))
            .unwrap();
        let size = || {
            s.host
                .run(&[
                    "display-message",
                    "-p",
                    "-t",
                    "=sized:",
                    "#{window_width}x#{window_height}",
                ])
                .unwrap()
                .trim()
                .to_owned()
        };
        assert_eq!(size(), "200x50");
        s.host.apply_options().unwrap();

        let mut client = s
            .host
            .command()
            .args(["-C", "attach", "-t", "=sized"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("control client");
        {
            let stdin = client.stdin.as_mut().unwrap();
            stdin.write_all(b"refresh-client -C 120,40\n").unwrap();
            stdin.flush().unwrap();
        }
        poll("window follows the client", || size() == "120x40");
        drop(client.stdin.take());
        let _ = client.kill();
        let _ = client.wait();
        s.host.kill(&id).unwrap();
    }

    /// A message of many paragraphs arrives whole: `cat` in the pane
    /// writes what it was sent to a file.
    #[test]
    fn long_input_is_typed_in_full() {
        let Some(s) = server() else { return };
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("typed.txt");
        let id = HostId("long".into());
        s.host
            .spawn(&SpawnSpec {
                command: Some(vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    // Canonical mode caps a line at 1024 bytes; a TUI turns it
                    // off, and so does this.
                    format!("stty -icanon; cat > {}", out.display()),
                ]),
                ..shell_spec("long", dir.path(), None)
            })
            .unwrap();
        let paragraph = "The quick brown fox jumps over the lazy dog. ".repeat(40);
        let text: String = (0..12)
            .map(|i| format!("Paragraph {i}: {paragraph}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.len() > 20_000);
        s.host.write_line(&id, &text).unwrap();
        // `cat` writes as lines complete; the Enter after the text ends the
        // last one.
        let mut last = String::new();
        let arrived = try_poll_on(Some(&s), &mut || {
            last = std::fs::read_to_string(&out).unwrap_or_default();
            last.trim_end() == text.trim_end()
        });
        if let Err(seen) = arrived {
            // The whole text is 21 kB; its length and tail say whether
            // the file stopped short or went wrong.
            let tail_from = last.floor_char_boundary(last.len().saturating_sub(200));
            panic!(
                "the whole text never reached the file: {} of {} bytes, ending {:?}; pane:\n{}{seen}",
                last.len(),
                text.len(),
                &last[tail_from..],
                s.host.snapshot(&id, Some(20)).unwrap_or_default()
            );
        }
        s.host.kill(&id).unwrap();
    }

    #[test]
    fn paste_body_normalises_breaks_and_drops_escape() {
        assert_eq!(
            paste_body("a\r\nb\rc\n\nd\x1b[201~e\tf é"),
            "a\nb\nc\n\nd[201~e\tf é"
        );
        assert_eq!(paste_body("plain"), "plain");
    }

    /// A pane running `cat -u` in raw mode, writing every byte it gets
    /// to a file, after enabling bracketed paste when `bracketed`. Waits
    /// until `cat` runs, so a send does not race the terminal setup.
    fn raw_cat(s: &Server, name: &str, dir: &Path, bracketed: bool) -> (HostId, PathBuf) {
        let out = dir.join(format!("{name}.bytes"));
        let id = HostId(name.into());
        let enable = if bracketed {
            "printf '\\033[?2004h'; "
        } else {
            ""
        };
        s.host
            .spawn(&SpawnSpec {
                command: Some(vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    format!("{enable}stty raw -echo; exec cat -u > {}", out.display()),
                ]),
                ..shell_spec(name, dir, None)
            })
            .unwrap();
        let target = pane_target(&id);
        let shown = |format: &str| {
            s.host
                .run(&["display-message", "-p", "-t", &target, format])
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        poll_on(Some(s), "cat in the pane", &mut || {
            shown("#{pane_current_command}") == "cat"
        });
        // Some supported tmux versions (the Linux runner's) predate this
        // format and print nothing for it; the bytes each test expects
        // still show which mode the pane was in.
        let flag = shown("#{bracket_paste_flag}");
        if !flag.is_empty() {
            assert_eq!(flag, if bracketed { "1" } else { "0" });
        }
        (id, out)
    }

    /// Waits until the file holds exactly `expected`, and panics with
    /// what it held instead.
    fn expect_bytes(s: &Server, out: &Path, expected: &[u8]) {
        let mut last = Vec::new();
        let arrived = try_poll_on(Some(s), &mut || {
            last = std::fs::read(out).unwrap_or_default();
            last == expected
        });
        if let Err(seen) = arrived {
            panic!(
                "expected {:?}, the pane got {:?}{seen}",
                String::from_utf8_lossy(expected),
                String::from_utf8_lossy(&last)
            );
        }
    }

    fn three_paragraphs() -> String {
        [
            "First paragraph, one line.",
            "Second paragraph\nwith a second line.",
            "Third paragraph.",
        ]
        .join("\n\n")
    }

    /// A message with blank lines reaches a pane that asked for bracketed
    /// paste as one paste, then one Enter after the closing marker: one
    /// submission. No buffer is left behind on the server.
    #[test]
    fn multi_paragraph_input_is_one_bracketed_paste() {
        let Some(s) = server() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (id, out) = raw_cat(&s, "bracketed", dir.path(), true);
        let text = three_paragraphs();
        s.host.write_line(&id, &text).unwrap();
        expect_bytes(&s, &out, format!("\x1b[200~{text}\x1b[201~\r").as_bytes());
        assert_eq!(s.host.run(&["list-buffers"]).unwrap(), "");
        s.host.kill(&id).unwrap();
    }

    /// A process that never enabled bracketed paste gets the text bare,
    /// with its line breaks as LF.
    #[test]
    fn multi_line_input_without_bracketed_paste_has_no_markers() {
        let Some(s) = server() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (id, out) = raw_cat(&s, "plain", dir.path(), false);
        let text = three_paragraphs();
        s.host.write_line(&id, &text).unwrap();
        expect_bytes(&s, &out, format!("{text}\r").as_bytes());
        s.host.kill(&id).unwrap();
    }

    /// A closing marker inside the text cannot end the paste early, and
    /// CR arrives as LF.
    #[test]
    fn escape_and_carriage_returns_in_a_paste_are_normalised() {
        let Some(s) = server() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (id, out) = raw_cat(&s, "escapes", dir.path(), true);
        s.host.write_line(&id, "a\r\nb\x1b[201~c\rd").unwrap();
        expect_bytes(&s, &out, b"\x1b[200~a\nb[201~c\nd\x1b[201~\r");
        s.host.kill(&id).unwrap();
    }

    /// One line is still typed, so it gets no paste markers even in a
    /// pane that asked for them.
    #[test]
    fn single_line_input_is_typed() {
        let Some(s) = server() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (id, out) = raw_cat(&s, "single", dir.path(), true);
        s.host.write_line(&id, "just one line").unwrap();
        expect_bytes(&s, &out, b"just one line\r");
        s.host.kill(&id).unwrap();
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
    fn spawn_keeps_env_values_off_every_command_line() {
        let Some(s) = server() else { return };
        // Built at run time so this file's own text can never match.
        let marker = format!(
            "argv-marker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let id = HostId("marked".into());
        let spec = SpawnSpec {
            id: id.clone(),
            cwd: std::env::temp_dir(),
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n' \"$SWITCHBOARD_TEST_MARKER\"; sleep 30".into(),
            ]),
            env: vec![("SWITCHBOARD_TEST_MARKER".into(), marker.clone())],
            scrollback: None,
        };
        s.host.spawn(&spec).unwrap();
        let pid = s.host.run(&["display-message", "-p", "#{pid}"]).unwrap();
        let pid = pid.trim();
        // The spawning client has exited by now; the long-lived server is
        // what a process listing keeps showing, and the unit test
        // `spawn_script_keeps_spec_off_the_argv` covers the client.
        match Command::new("ps")
            .args(["-o", "command=", "-p", pid])
            .output()
        {
            Ok(out) if out.status.success() => {
                let argv = String::from_utf8_lossy(&out.stdout);
                assert!(argv.contains("start-server"), "{argv}");
                assert!(!argv.contains(&marker), "server argv leaks: {argv}");
            }
            other => eprintln!("skipping the server argv check: ps failed: {other:?}"),
        }
        poll_on(Some(&s), "the marker in the pane", &mut || {
            s.host.snapshot(&id, None).unwrap().contains(&marker)
        });
        s.host.kill(&id).unwrap();
    }

    #[test]
    fn spawn_env_value_round_trips_through_tmux_quoting() {
        let Some(s) = server() else { return };
        // A line starting with `#` is a tmux comment even inside quotes,
        // so the headings pin that line breaks never reach the parser.
        let value = "it's \"q\" $HOME ~ ; \\; #{session_name} back\\slash\n\
                     # Heading\n  ## Section\n~ last line";
        let id = HostId("quoted".into());
        // The pane prints both the argument and the variable, and the
        // assertions read its screen. `show-environment` is not the oracle:
        // tmux 3.4 vis-encodes a command's output to its client, so `$HOME`
        // prints as `\$HOME` there while the pane's environment holds the
        // value exactly (3.5 stopped encoding; `capture-pane` never did).
        let spec = SpawnSpec {
            id: id.clone(),
            cwd: std::env::temp_dir(),
            command: Some(vec![
                "sh".into(),
                "-c".into(),
                "printf '<%s>[%s]' \"$1\" \"$SWITCHBOARD_TEST_QUOTED\"; sleep 30".into(),
                "sh".into(),
                value.into(),
            ]),
            env: vec![("SWITCHBOARD_TEST_QUOTED".into(), value.into())],
            scrollback: None,
        };
        s.host.spawn(&spec).unwrap();
        let first = "it's \"q\" $HOME ~ ; \\; #{session_name} back\\slash";
        poll_on(Some(&s), "the quoted command's output", &mut || {
            let shot = s.host.snapshot(&id, None).unwrap();
            shot.contains(&format!("<{first}"))
                && shot.contains("# Heading")
                && shot.contains("  ## Section")
                && shot.contains(&format!("~ last line>[{first}"))
                && shot.contains("~ last line]")
        });
        s.host.kill(&id).unwrap();
    }

    #[test]
    fn command_exit_code_is_reported() {
        let Some(s) = server() else { return };
        let id = HostId("job".into());
        let spec = SpawnSpec {
            id: id.clone(),
            cwd: std::env::temp_dir(),
            // Not a bare `exit`: a command that lives into the pane's
            // first read exits through the ordinary path.
            command: Some(vec!["sh".into(), "-c".into(), "sleep 0.3; exit 3".into()]),
            env: Vec::new(),
            scrollback: None,
        };
        s.host.spawn(&spec).unwrap();
        poll_on(Some(&s), "the pane's exit", &mut || {
            matches!(
                s.host.status(&id).unwrap().liveness,
                Liveness::Exited { .. }
            )
        });
        // tmux can report the death a moment before its status.
        let with_code = poll_for(Some(&s), Duration::from_secs(5), &mut || {
            s.host.status(&id).unwrap().liveness == Liveness::Exited { code: Some(3) }
        });
        if let Err(seen) = with_code {
            // An unreadable version takes the strict path: comparing the
            // `Option`s would let `None < Some((3, 5))` pass silently.
            let old_tmux = s
                .host
                .probe()
                .ok()
                .and_then(|info| parse_version(&info.description))
                .is_some_and(|v| v < (3, 5));
            if old_tmux && s.host.status(&id).unwrap().liveness == (Liveness::Exited { code: None })
            {
                eprintln!("this tmux left the pane dead without a status{seen}");
                return;
            }
            panic!("timed out waiting for exit status 3{seen}");
        }
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
