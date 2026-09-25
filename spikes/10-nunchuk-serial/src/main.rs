//! Read the Feather's button and stick lines, print each with a
//! timestamp, ping it once a second to measure the round trip, and
//! survive the cable being pulled.
//!
//! Run with `firmware/nunchuk/code.py` on the device:
//!
//! ```sh
//! cargo run --release            # first /dev/cu.usbmodem*
//! cargo run --release -- /dev/cu.usbmodem11301
//! ```

use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

fn port_path(arg: Option<String>) -> Option<String> {
    if let Some(path) = arg {
        return Some(path);
    }
    let mut names: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .filter(|n| n.contains("usbmodem"))
        .map(|n| n.replace("/dev/tty.", "/dev/cu."))
        .collect();
    names.sort();
    names.dedup();
    names.into_iter().next()
}

fn main() {
    let arg = std::env::args().nth(1);
    let started = Instant::now();
    loop {
        let Some(path) = port_path(arg.clone()) else {
            println!("[{:8.3}] no usbmodem port; retrying", started.elapsed().as_secs_f64());
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        match serialport::new(&path, 115_200)
            .timeout(Duration::from_millis(200))
            .open()
        {
            Ok(port) => {
                println!("[{:8.3}] opened {path}", started.elapsed().as_secs_f64());
                session(port, started);
            }
            Err(e) => println!("[{:8.3}] open {path}: {e}", started.elapsed().as_secs_f64()),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Reads lines until the port fails. A read timeout is not a failure:
/// it is the moment to send the next ping.
fn session(port: Box<dyn serialport::SerialPort>, started: Instant) {
    let mut writer = match port.try_clone() {
        Ok(w) => w,
        Err(e) => {
            println!("clone: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(port);
    let mut line = String::new();
    let mut ping_sent: Option<Instant> = None;
    let mut last_ping: Option<Instant> = None;
    loop {
        if last_ping.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) && ping_sent.is_none() {
            if writer.write_all(b"P\n").is_err() {
                return;
            }
            ping_sent = Some(Instant::now());
            last_ping = Some(Instant::now());
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                println!("[{:8.3}] eof", started.elapsed().as_secs_f64());
                return;
            }
            Ok(_) => {
                let text = line.trim();
                if text == "P" {
                    if let Some(sent) = ping_sent.take() {
                        println!(
                            "[{:8.3}] pong {:.1} ms",
                            started.elapsed().as_secs_f64(),
                            sent.elapsed().as_secs_f64() * 1000.0
                        );
                    }
                } else {
                    println!("[{:8.3}] {text}", started.elapsed().as_secs_f64());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if ping_sent.is_some_and(|s| s.elapsed() > Duration::from_secs(2)) {
                    println!("[{:8.3}] ping lost", started.elapsed().as_secs_f64());
                    ping_sent = None;
                }
            }
            Err(e) => {
                println!("[{:8.3}] read: {e}", started.elapsed().as_secs_f64());
                return;
            }
        }
    }
}
