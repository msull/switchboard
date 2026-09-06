//! Q5 detail for tmux: which of the three ways to hand a session its
//! environment actually reach the shell, and how the global environment
//! of a long-lived server drifts from the app that started it.

use spike_process_host::{TEST_VAR, Tmux, pause};

fn main() {
    let tmux = Tmux::new("env");
    tmux.run(&["kill-server"]).ok();

    println!("== 1. the server inherits the environment of whoever starts it");
    // The first command on a socket starts the server; its environment is the
    // server's global environment for the rest of its life.
    let mut cmd = tmux.command();
    cmd.env("SWITCHBOARD_SERVER_BORN_WITH", "yes");
    cmd.args(["new-session", "-d", "-s", "a"]);
    assert!(cmd.status().expect("start server").success());
    tmux.show(&["show-environment", "-g", "SWITCHBOARD_SERVER_BORN_WITH"])
        .ok();

    println!("\n== 2. new-session -e sets a per-session variable (tmux >= 3.2)");
    tmux.show(&[
        "new-session",
        "-d",
        "-s",
        "b",
        "-e",
        &format!("{TEST_VAR}=via-dash-e"),
    ])
    .expect("new-session");
    tmux.show(&["show-environment", "-t", "b", TEST_VAR]).ok();

    println!(
        "\n== 3. set-environment on a session affects panes created afterwards, not existing ones"
    );
    tmux.show(&[
        "set-environment",
        "-t",
        "a",
        TEST_VAR,
        "via-set-environment",
    ])
    .expect("set-environment");
    tmux.show(&["new-window", "-d", "-t", "a", "-n", "later"])
        .expect("new-window");
    pause(500);
    for target in ["a:0", "a:later", "b"] {
        tmux.run(&[
            "send-keys",
            "-t",
            target,
            &format!("echo {target} VAR=${TEST_VAR}"),
            "Enter",
        ])
        .expect("send-keys");
    }
    pause(500);
    for target in ["a:0", "a:later", "b"] {
        tmux.show(&["capture-pane", "-p", "-t", target])
            .map(|_| ())
            .ok();
    }

    println!(
        "\n== 4. update-environment: what tmux copies from a *client* on attach (default list)"
    );
    tmux.show(&["show-options", "-g", "update-environment"])
        .ok();
    println!(
        "  Only an attaching client updates these; detached `send-keys` never touches the env."
    );

    tmux.run(&["kill-server"]).ok();
}
