//! Shared helpers for the process-host spike. Everything experiment-specific
//! lives in `examples/`; this is only the plumbing they have in common.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Every tmux server this spike starts lives on a socket with this prefix, so
/// it can never collide with the user's own `tmux` sessions.
pub const SOCKET_PREFIX: &str = "switchboard-spike";

/// Environment variable the hosted shells must see; proves env injection.
pub const TEST_VAR: &str = "SWITCHBOARD_TEST";

/// The tmux binary: `SWITCHBOARD_TMUX` if set (the spike ran against a
/// relocated Homebrew bottle), otherwise `tmux` from `PATH`.
#[must_use]
pub fn tmux_bin() -> String {
    std::env::var("SWITCHBOARD_TMUX").unwrap_or_else(|_| "tmux".to_owned())
}

/// Scratch directory for scrollback files and the like: `target/spike-out`.
#[must_use]
pub fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/spike-out");
    std::fs::create_dir_all(&dir).expect("create output dir");
    dir
}

/// A handle to one private tmux server, addressed by socket name.
pub struct Tmux {
    pub socket: String,
}

impl Tmux {
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            socket: format!("{SOCKET_PREFIX}-{name}"),
        }
    }

    /// A `Command` for this server. `-f /dev/null` keeps the user's config out
    /// of the experiment; the real app would pass its own config file here.
    #[must_use]
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(tmux_bin());
        cmd.args(["-L", &self.socket, "-f", "/dev/null"]);
        cmd
    }

    /// Run one tmux command and return trimmed stdout, or stderr on failure.
    pub fn run(&self, args: &[&str]) -> Result<String, String> {
        let out = self
            .command()
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim_end().to_owned())
        }
    }

    /// Like `run` but prints the command line and its result, for the README.
    pub fn show(&self, args: &[&str]) -> Result<String, String> {
        println!("$ tmux -L {} {}", self.socket, args.join(" "));
        let result = self.run(args);
        match &result {
            Ok(s) if s.is_empty() => println!("  (ok, no output)"),
            Ok(s) => println!("{}", indent(s)),
            Err(e) => println!("  ERROR: {e}"),
        }
        result
    }
}

/// Indent every line of a block by two spaces so command output is easy to
/// tell apart from the commands themselves.
#[must_use]
pub fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `f` `n` times and report min / mean / max wall time.
pub fn bench(label: &str, n: u32, mut f: impl FnMut()) {
    let mut samples = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let t = Instant::now();
        f();
        samples.push(t.elapsed());
    }
    let min = samples.iter().min().copied().unwrap_or_default();
    let max = samples.iter().max().copied().unwrap_or_default();
    let mean = samples.iter().sum::<Duration>() / n;
    println!("{label}: n={n} min={min:.2?} mean={mean:.2?} max={max:.2?}");
}

/// Sleep helper so the examples read as a script.
pub fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}
