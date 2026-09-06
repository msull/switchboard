//! Q2 part 1, Q5, Q6 for tmux: start a private tmux server, spawn a named
//! session with a cwd and an env var, pipe its scrollback to disk, send keys,
//! capture the pane, check liveness and exit status, and time a capture
//! round trip. Leaves the session running so `tmux_reattach` can find it.

use spike_process_host::{TEST_VAR, Tmux, bench, out_dir, pause};

// The experiment is a script that reads top to bottom; splitting it would
// hide the order the commands run in.
#[allow(clippy::too_many_lines)]
fn main() {
    let tmux = Tmux::new("host");
    let session = "sess";
    let cwd = std::env::temp_dir();
    let cwd = cwd.to_str().expect("utf8 temp dir");
    let scrollback = out_dir().join("tmux-scrollback.log");
    let _ = std::fs::remove_file(&scrollback);

    println!("== tmux version");
    tmux.show(&["-V"]).ok();

    println!("\n== spawn: new-session with cwd and env");
    // `-e` sets a variable in the new session's environment (tmux >= 3.2).
    // `-x/-y` size the detached session; otherwise it is 80x24.
    tmux.show(&["kill-server"]).ok(); // clean slate; ignore "no server running"
    tmux.show(&[
        "new-session",
        "-d",
        "-s",
        session,
        "-c",
        cwd,
        "-e",
        &format!("{TEST_VAR}=hello-from-tmux"),
        "-x",
        "120",
        "-y",
        "40",
    ])
    .expect("new-session");

    println!("\n== liveness: list-panes");
    let fmt =
        "pid=#{pane_pid} dead=#{pane_dead} cmd=#{pane_current_command} path=#{pane_current_path}";
    tmux.show(&["list-panes", "-t", session, "-F", fmt])
        .expect("list-panes");

    println!("\n== scrollback to disk: pipe-pane -o");
    let pipe = format!("cat >> '{}'", scrollback.display());
    tmux.show(&["pipe-pane", "-o", "-t", session, &pipe])
        .expect("pipe-pane");

    println!("\n== send-keys: env var, cwd, login/interactive flags, .zshrc marker");
    // SAM_CLI_TELEMETRY is exported by the user's .zshrc, so seeing it proves
    // the interactive init ran; `[[ -o login ]]` proves it is a login shell.
    let probe = format!(
        "echo VAR=${TEST_VAR}; echo CWD=$PWD; [[ -o login ]] && echo LOGIN=yes; \
         [[ -o interactive ]] && echo INTERACTIVE=yes; echo ZSHRC_MARKER=$SAM_CLI_TELEMETRY; \
         echo SHELL_PID=$$"
    );
    tmux.show(&["send-keys", "-t", session, &probe, "Enter"])
        .expect("send-keys");
    pause(700);

    println!("\n== capture-pane -p -S - (whole scrollback)");
    tmux.show(&["capture-pane", "-p", "-S", "-", "-t", session])
        .expect("capture-pane");

    println!("\n== latency");
    bench("capture-pane -p -S -", 50, || {
        tmux.run(&["capture-pane", "-p", "-S", "-", "-t", session])
            .expect("capture");
    });
    bench("list-panes -F", 50, || {
        tmux.run(&["list-panes", "-t", session, "-F", fmt])
            .expect("list-panes");
    });
    bench("send-keys (no wait for output)", 50, || {
        tmux.run(&["send-keys", "-t", session, "", ""])
            .expect("send-keys");
    });

    println!(
        "\n== latency end to end: send-keys to a `cat` window, poll capture-pane until the echo shows"
    );
    tmux.show(&["new-window", "-d", "-t", session, "-n", "cat", "cat"])
        .expect("new-window");
    pause(200);
    bench("send-keys + poll capture-pane until visible", 50, || {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let i = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let line = format!("raw{i}");
        tmux.run(&["send-keys", "-t", "sess:cat", &line, "Enter"])
            .expect("send-keys");
        // tty echo + cat output = the line appears twice once cat has answered
        while tmux
            .run(&["capture-pane", "-p", "-t", "sess:cat"])
            .expect("capture")
            .matches(&line)
            .count()
            < 2
        {}
    });
    tmux.show(&["kill-window", "-t", "sess:cat"])
        .expect("kill-window");

    println!("\n== exit detection: a window whose command exits with code 3");
    // remain-on-exit keeps the dead pane around so pane_dead / pane_dead_status
    // are readable; without it the window vanishes and only a hook would tell.
    tmux.show(&["set-option", "-gw", "remain-on-exit", "on"])
        .expect("set-option");
    tmux.show(&[
        "new-window",
        "-d",
        "-t",
        session,
        "-n",
        "job",
        "sh -c 'echo working; sleep 1; exit 3'",
    ])
    .expect("new-window");
    pause(300);
    let fmt2 = "win=#{window_name} pid=#{pane_pid} dead=#{pane_dead} status=#{pane_dead_status} cmd=#{pane_current_command}";
    tmux.show(&["list-panes", "-s", "-t", session, "-F", fmt2])
        .expect("list-panes");
    pause(1200);
    tmux.show(&["list-panes", "-s", "-t", session, "-F", fmt2])
        .expect("list-panes");

    println!("\n== scrollback file on disk");
    println!("$ cat {}", scrollback.display());
    match std::fs::read_to_string(&scrollback) {
        Ok(s) => println!("{}", spike_process_host::indent(&s)),
        Err(e) => println!("  ERROR: {e}"),
    }

    println!(
        "\n== done; session '{session}' left running on socket {}",
        tmux.socket
    );
    println!("   now run: cargo run --example tmux_reattach");
}
