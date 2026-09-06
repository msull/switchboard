//! Q4, Q5, Q6 for app-owned PTYs: open a pty with `portable-pty`, spawn the
//! user's login shell with a cwd and env var, tee output to disk, write a
//! command, resize, detect exit. Then spawn two orphans and exit without
//! waiting, to show what happens to children when the host process dies.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use spike_process_host::{TEST_VAR, out_dir, pause};

fn argv(args: &[&str]) -> CommandBuilder {
    CommandBuilder::from_argv(args.iter().map(std::convert::Into::into).collect())
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

// The experiment is a script that reads top to bottom; splitting it would
// hide the order the commands run in.
#[allow(clippy::too_many_lines)]
fn main() {
    let scrollback = out_dir().join("pty-scrollback.log");
    let _ = std::fs::remove_file(&scrollback);
    let pty = native_pty_system();

    println!("== spawn login shell in a pty with cwd + env");
    let pair = pty.openpty(size(24, 80)).expect("openpty");
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.arg("-l"); // login shell; interactive is implied by the pty
    cmd.cwd(std::env::temp_dir());
    cmd.env(TEST_VAR, "hello-from-pty");
    cmd.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave); // the child owns the slave now; keeping it open here would mask EOF
    println!("  shell={shell} pid={:?}", child.process_id());

    // Reader thread: everything the shell prints goes to a buffer for us and
    // to the scrollback file (the tee). `Arc<Mutex<..>>` shares the buffer
    // between threads; the pty reader is a plain `Read`, so std io works.
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let buf: Arc<Mutex<Vec<u8>>> = Arc::default();
    let tee = buf.clone();
    let mut file = std::fs::File::create(&scrollback).expect("scrollback file");
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break, // EOF: the slave side closed, i.e. the shell exited
                Ok(n) => {
                    file.write_all(&chunk[..n]).expect("tee write");
                    tee.lock().expect("lock").extend_from_slice(&chunk[..n]);
                }
            }
        }
    });
    let mut writer = pair.master.take_writer().expect("writer");

    let snapshot = |since: usize| -> String {
        let b = buf.lock().expect("lock");
        String::from_utf8_lossy(&b[since.min(b.len())..]).into_owned()
    };
    let wait_for = |needle: &str, timeout: Duration| -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if snapshot(0).contains(needle) {
                return true;
            }
            pause(5);
        }
        false
    };

    pause(600); // let the shell finish its init and draw a prompt
    println!("\n== write probe command");
    writeln!(
        writer,
        "echo VAR=${TEST_VAR}; echo CWD=$PWD; [[ -o login ]] && echo LOGIN=yes; \
         [[ -o interactive ]] && echo INTERACTIVE=yes; echo ZSHRC_MARKER=$SAM_CLI_TELEMETRY; \
         echo SIZE=$(stty size); echo END1"
    )
    .expect("write");
    let ok = wait_for("END1", Duration::from_secs(3));
    println!("  got END1: {ok}");

    println!("\n== resize to 50x200, then ask the shell what it sees");
    pair.master.resize(size(50, 200)).expect("resize");
    writeln!(writer, "echo SIZE=$(stty size); echo END2").expect("write");
    let ok = wait_for("END2", Duration::from_secs(3));
    println!("  got END2: {ok}");

    println!("\n== latency: write `echo ping<N>` and wait until the reply lands in the buffer");
    // `po''ng` keeps the typed echo from matching the needle; only the shell's
    // real output contains `pong<N>`. Polling sleeps a little each time: a
    // hot spin on the mutex can starve the reader thread that fills it.
    let mut samples = Vec::new();
    for i in 0..50 {
        let mark = buf.lock().expect("lock").len();
        let t = Instant::now();
        writeln!(writer, "echo po''ng{i}").expect("write");
        let needle = format!("pong{i}");
        while !snapshot(mark).contains(&needle) {
            assert!(
                t.elapsed() < Duration::from_secs(5),
                "no reply to echo pong{i}"
            );
            std::thread::sleep(Duration::from_micros(100));
        }
        samples.push(t.elapsed());
    }
    let min = samples.iter().min().copied().unwrap_or_default();
    let max = samples.iter().max().copied().unwrap_or_default();
    let mean = samples.iter().sum::<Duration>() / 50;
    println!("  write->echo round trip: n=50 min={min:.2?} mean={mean:.2?} max={max:.2?}");

    println!("\n== latency without a shell: a pty running `cat`, write a line, wait for its echo");
    {
        let pair = pty.openpty(size(24, 80)).expect("openpty");
        let mut cat = pair.slave.spawn_command(argv(&["cat"])).expect("spawn cat");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("reader");
        let mut writer = pair.master.take_writer().expect("writer");
        let mut samples = Vec::new();
        let mut chunk = [0u8; 256];
        for i in 0..50 {
            let t = Instant::now();
            writeln!(writer, "raw{i}").expect("write");
            // The tty echoes the line once and cat prints it once: two copies.
            let mut got = Vec::new();
            let needle = format!("raw{i}");
            while got
                .windows(needle.len())
                .filter(|w| *w == needle.as_bytes())
                .count()
                < 2
            {
                let n = reader.read(&mut chunk).expect("read");
                got.extend_from_slice(&chunk[..n]);
            }
            samples.push(t.elapsed());
        }
        let min = samples.iter().min().copied().unwrap_or_default();
        let max = samples.iter().max().copied().unwrap_or_default();
        let mean = samples.iter().sum::<Duration>() / 50;
        println!("  write->cat echo round trip: n=50 min={min:.2?} mean={mean:.2?} max={max:.2?}");
        cat.kill().expect("kill cat");
        cat.wait().expect("wait cat");
    }

    println!("\n== exit detection");
    println!(
        "  before exit: try_wait = {:?}",
        child.try_wait().map(|s| s.map(|s| s.exit_code()))
    );
    writeln!(writer, "exit 7").expect("write");
    let status = child.wait().expect("wait");
    println!(
        "  after `exit 7`: wait = exit_code {} success={}",
        status.exit_code(),
        status.success()
    );

    pause(100);
    println!("\n== raw pty output (escape sequences shown as-is)");
    let text = snapshot(0);
    println!(
        "{}",
        spike_process_host::indent(&text.replace('\u{1b}', "\\e"))
    );
    println!(
        "\n== scrollback file: {} ({} bytes)",
        scrollback.display(),
        std::fs::metadata(&scrollback).map_or(0, |m| m.len())
    );

    println!("\n== orphan test: spawn two children and exit without waiting");
    // A plain child: closing the master sends SIGHUP to the pty's foreground
    // process group, so it should be gone right after this program exits.
    let plain = pty.openpty(size(24, 80)).expect("openpty");
    let plain_child = plain
        .slave
        .spawn_command(argv(&["sleep", "300"]))
        .expect("spawn");
    // A child that ignores SIGHUP survives, but its pty is gone: nothing can
    // ever read its output again. That is the "no reattach" case.
    let tough = pty.openpty(size(24, 80)).expect("openpty");
    let tough_child = tough
        .slave
        .spawn_command(argv(&["sh", "-c", "trap '' HUP; exec sleep 300"]))
        .expect("spawn");
    println!(
        "  plain child pid: {:?}  (expected to die with the host)",
        plain_child.process_id()
    );
    println!(
        "  HUP-ignoring child pid: {:?}  (expected to survive, orphaned, unreachable)",
        tough_child.process_id()
    );
    println!("  host exiting now; check with: ps -o pid,ppid,stat,tty,command -p <pids>");
    // Pids go to a file so the README's check step can find them.
    std::fs::write(
        out_dir().join("orphan-pids"),
        format!(
            "{} {}\n",
            plain_child.process_id().unwrap_or(0),
            tough_child.process_id().unwrap_or(0)
        ),
    )
    .expect("write pids");
}
