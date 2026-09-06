//! Q2 part 2: a fresh process finds the session `tmux_host` left behind,
//! reads its scrollback, sends more input, then kills the server.
//! Set `KEEP=1` to leave the server running.

use spike_process_host::{Tmux, pause};

fn main() {
    let tmux = Tmux::new("host");
    let session = "sess";

    println!("== has-session (exit status is the answer)");
    match tmux.show(&["has-session", "-t", session]) {
        Ok(_) => println!("  session '{session}' is alive from a previous process"),
        Err(e) => {
            println!("  no session: {e}\n  run `cargo run --example tmux_host` first");
            return;
        }
    }

    println!("\n== the pane still has its history and its shell");
    tmux.show(&[
        "list-panes",
        "-s",
        "-t",
        session,
        "-F",
        "win=#{window_name} pid=#{pane_pid} dead=#{pane_dead} cmd=#{pane_current_command}",
    ])
    .expect("list-panes");
    tmux.show(&[
        "send-keys",
        "-t",
        session,
        "echo REATTACHED_PID=$$",
        "Enter",
    ])
    .expect("send-keys");
    pause(400);
    tmux.show(&["capture-pane", "-p", "-S", "-", "-t", session])
        .expect("capture-pane");

    println!("\n== server-wide view: what an app would enumerate on startup");
    tmux.show(&[
        "list-sessions",
        "-F",
        "#{session_name} created=#{session_created} attached=#{session_attached}",
    ])
    .expect("list-sessions");

    if std::env::var_os("KEEP").is_some() {
        println!("\nKEEP set; leaving server on socket {}", tmux.socket);
        return;
    }
    println!("\n== kill-server, then has-session again");
    tmux.show(&["kill-server"]).expect("kill-server");
    match tmux.show(&["has-session", "-t", session]) {
        Ok(_) => println!("  still there?!"),
        Err(_) => println!("  gone, as expected"),
    }
}
