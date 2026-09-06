//! Q3: drive tmux control mode (`tmux -C`) from a long-lived child process.
//! Attach over pipes, send `echo hello`, parse `%begin`/`%end` blocks and
//! `%output` notifications, decode the octal escapes, then tear down.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use spike_process_host::{TEST_VAR, Tmux, pause};

/// One line from the control client, classified.
enum Msg {
    Begin { id: u64 },
    End { id: u64, error: bool },
    Output { pane: String, text: String },
    Other(String),
    Body(String),
}

fn parse(line: &str) -> Msg {
    let mut parts = line.splitn(4, ' ');
    match parts.next() {
        // "%begin <timestamp> <command-number> <flags>"
        Some("%begin") => Msg::Begin {
            id: nth_u64(line, 2),
        },
        Some("%end") => Msg::End {
            id: nth_u64(line, 2),
            error: false,
        },
        Some("%error") => Msg::End {
            id: nth_u64(line, 2),
            error: true,
        },
        // "%output %<pane-id> <escaped bytes>"
        Some("%output") => {
            let pane = parts.next().unwrap_or_default().to_owned();
            let rest = line.splitn(3, ' ').nth(2).unwrap_or_default();
            Msg::Output {
                pane,
                text: unescape(rest),
            }
        }
        Some(s) if s.starts_with('%') => Msg::Other(line.to_owned()),
        _ => Msg::Body(line.to_owned()),
    }
}

fn nth_u64(line: &str, n: usize) -> u64 {
    line.split(' ')
        .nth(n)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// tmux escapes every byte outside printable ASCII (and `\` itself) as a
/// three-digit octal sequence, e.g. `\033[0m` and `\015\012` for CR LF.
fn unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4].iter().all(u8::is_ascii_digit)
        {
            let oct = std::str::from_utf8(&bytes[i + 1..i + 4]).expect("digits");
            out.push(u8::from_str_radix(oct, 8).expect("octal"));
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// The experiment is a script that reads top to bottom; splitting it would
// hide the order the commands run in.
#[allow(clippy::too_many_lines)]
fn main() {
    let tmux = Tmux::new("control");
    let session = "ctl";
    tmux.run(&["kill-server"]).ok();
    tmux.show(&[
        "new-session",
        "-d",
        "-s",
        session,
        "-x",
        "100",
        "-y",
        "20",
        "-e",
        &format!("{TEST_VAR}=hello-from-control"),
    ])
    .expect("new-session");

    println!("\n== spawn `tmux -C attach` with stdin/stdout as pipes");
    let mut child = tmux
        .command()
        .args(["-C", "attach-session", "-t", session])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn control client");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");

    // Reader thread: one line per message, forwarded over a channel so the
    // main thread can wait with a timeout instead of blocking on the pipe.
    // Lines are read as bytes, not `String`s: `BufRead::lines` fails on a
    // line that is not valid UTF-8, and a `%output` chunk can end anywhere.
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) => {
                    eprintln!("  [reader] EOF on control client stdout");
                    break;
                }
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line)
                        .trim_end_matches(['\r', '\n'])
                        .to_owned();
                    if tx.send(text).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("  [reader] error: {e}");
                    break;
                }
            }
        }
    });

    let mut output_text = String::new();
    let drain = |label: &str, until: Duration, output_text: &mut String| {
        let start = Instant::now();
        let deadline = start + until;
        println!("-- {label}");
        while let Ok(line) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            let msg = parse(&line);
            match &msg {
                Msg::Output { text, .. } => output_text.push_str(text),
                Msg::Body(_) | Msg::Begin { .. } | Msg::End { .. } | Msg::Other(_) => {}
            }
            println!("  +{:>8.2?} raw: {line}", start.elapsed());
            let decoded = match &msg {
                Msg::Begin { id } => format!("begin of reply #{id}"),
                Msg::End { id, error } => format!("end of reply #{id}, error={error}"),
                Msg::Output { pane, text } => format!("output from pane {pane}: {text:?}"),
                Msg::Other(s) => format!("notification: {s}"),
                Msg::Body(s) => format!("reply body: {s}"),
            };
            println!("                -> {decoded}");
        }
    };

    drain(
        "greeting on attach",
        Duration::from_millis(400),
        &mut output_text,
    );

    println!("\n== send a command; time until its %end");
    let t = Instant::now();
    writeln!(
        stdin,
        "send-keys -t {session} 'echo hello; echo VAR=${TEST_VAR}' Enter"
    )
    .expect("write");
    stdin.flush().expect("flush");
    drain(
        "response + %output notifications",
        Duration::from_millis(600),
        &mut output_text,
    );
    println!(
        "  (drain window was 600ms, {:.2?} total; the +offsets show when each line arrived)",
        t.elapsed()
    );

    println!("\n== a query command whose reply is a %begin/%end body, not a notification");
    writeln!(stdin, "list-panes -t {session} -F 'pid=#{{pane_pid}} dead=#{{pane_dead}} cmd=#{{pane_current_command}}'").expect("write");
    stdin.flush().expect("flush");
    drain(
        "list-panes reply",
        Duration::from_millis(300),
        &mut output_text,
    );

    println!("\n== an invalid command -> %error block");
    writeln!(stdin, "no-such-command").expect("write");
    stdin.flush().expect("flush");
    drain("error reply", Duration::from_millis(300), &mut output_text);

    println!("\n== decoded %output stream so far:");
    println!("{}", spike_process_host::indent(&output_text));

    println!("\n== detach: closing stdin alone does NOT end `tmux -C attach`; send detach-client");
    drop(stdin);
    pause(300);
    println!(
        "  after closing stdin: try_wait = {:?}",
        child.try_wait().map(|s| s.map(|s| s.code()))
    );
    if child.try_wait().expect("try_wait").is_none() {
        // A fresh client can detach the stuck one; the real app would write
        // `detach-client` on the control stdin before closing it.
        tmux.show(&["detach-client", "-t", session]).ok();
        pause(300);
        println!(
            "  after detach-client: try_wait = {:?}",
            child.try_wait().map(|s| s.map(|s| s.code()))
        );
    }
    if child.try_wait().expect("try_wait").is_none() {
        child.kill().expect("kill");
        println!("  killed the control client");
    }
    drain(
        "final notifications",
        Duration::from_millis(200),
        &mut output_text,
    );
    tmux.show(&["has-session", "-t", session])
        .expect("session survives detach");
    tmux.show(&["kill-server"]).expect("kill-server");
}
