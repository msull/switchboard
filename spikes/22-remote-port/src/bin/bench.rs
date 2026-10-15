//! The client half of the spike: a pinned TLS client that measures the
//! server of `src/main.rs` the way docs/remote-client.md §10 asks.
//!
//! Run the server first, then
//! `cargo run --release --bin bench -- [--port 43187] [--pin-file pin.txt] [--phase all|latency|stall|overflow|pin]`.

use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use spike_remote_port::{Pin, provider};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

const TOKEN: &str = "Bearer spike-token";

struct Args {
    port: u16,
    pin_file: String,
    phase: String,
}

fn args() -> Args {
    let mut a = Args {
        port: 43187,
        pin_file: "pin.txt".into(),
        phase: "all".into(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--port" => a.port = it.next().unwrap().parse().unwrap(),
            "--pin-file" => a.pin_file = it.next().unwrap(),
            "--phase" => a.phase = it.next().unwrap(),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

fn pin_from_hex(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    out
}

fn client_config(fingerprint: [u8; 32]) -> Arc<ClientConfig> {
    let provider = provider();
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pin {
            fingerprint,
            provider,
        }))
        .with_no_client_auth();
    Arc::new(config)
}

type Tls = StreamOwned<ClientConnection, TcpStream>;

fn connect(port: u16, config: &Arc<ClientConfig>) -> std::io::Result<Tls> {
    let tcp = TcpStream::connect(("127.0.0.1", port))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    tcp.set_nodelay(true)?;
    let conn = ClientConnection::new(
        Arc::clone(config),
        ServerName::try_from("localhost").unwrap(),
    )
    .map_err(std::io::Error::other)?;
    let mut s = StreamOwned::new(conn, tcp);
    while s.conn.is_handshaking() {
        s.conn.complete_io(&mut s.sock)?;
    }
    Ok(s)
}

/// One request on an open connection: the status and the body.
fn request(s: &mut Tls, path: &str, token: Option<&str>) -> std::io::Result<(u16, String)> {
    let auth = token
        .map(|t| format!("Authorization: {t}\r\n"))
        .unwrap_or_default();
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n{auth}\r\n")?;
    s.flush()?;
    let mut reader = BufReader::new(s);
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, line.clone()))?;
    let mut length = 0usize;
    loop {
        let mut h = String::new();
        std::io::BufRead::read_line(&mut reader, &mut h)?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some(v) = h.strip_prefix("Content-Length:") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// The sample at `permille` of the way through `sorted`.
fn percentile(sorted: &[Duration], permille: usize) -> Duration {
    sorted[(sorted.len() - 1) * permille / 1000]
}

fn report(name: &str, mut samples: Vec<Duration>) {
    samples.sort();
    println!(
        "{name}: n={} p50={:?} p99={:?} max={:?}",
        samples.len(),
        percentile(&samples, 500),
        percentile(&samples, 990),
        samples.last().unwrap()
    );
}

/// Request latency through the frame handoff: on one kept-alive
/// connection, and with a fresh TLS handshake each time.
fn latency(port: u16, config: &Arc<ClientConfig>) {
    let mut s = connect(port, config).unwrap();
    let mut keep = Vec::new();
    for _ in 0..300 {
        let t0 = Instant::now();
        let (status, _) = request(&mut s, "/api/v1/info", Some(TOKEN)).unwrap();
        assert_eq!(status, 200);
        keep.push(t0.elapsed());
    }
    report("info on a kept connection", keep);
    let mut fresh = Vec::new();
    for _ in 0..50 {
        let t0 = Instant::now();
        let mut s = connect(port, config).unwrap();
        let (status, _) = request(&mut s, "/api/v1/info", Some(TOKEN)).unwrap();
        assert_eq!(status, 200);
        fresh.push(t0.elapsed());
    }
    report("info with a fresh TLS connection", fresh);
}

/// Twenty connections that never handshake, held open, and a healthy
/// paired client measured against them: refused at once while the
/// unauthenticated pool is full, served once the stalled ones time out.
fn stall(port: u16, config: &Arc<ClientConfig>) {
    let mut established = connect(port, config).unwrap();
    let (status, _) = request(&mut established, "/api/v1/info", Some(TOKEN)).unwrap();
    assert_eq!(status, 200);
    let mut held = Vec::new();
    for _ in 0..20 {
        held.push(TcpStream::connect(("127.0.0.1", port)).unwrap());
    }
    thread::sleep(Duration::from_millis(200));
    let t0 = Instant::now();
    let (status, _) = request(&mut established, "/api/v1/info", Some(TOKEN)).unwrap();
    println!(
        "stall: the already-paired connection answered {status} in {:?} with 20 stalled connections held",
        t0.elapsed()
    );
    let t0 = Instant::now();
    let fresh =
        connect(port, config).and_then(|mut s| request(&mut s, "/api/v1/info", Some(TOKEN)));
    println!(
        "stall: a new connection during the stall: {} after {:?}",
        match &fresh {
            Ok((status, _)) => format!("served {status}"),
            Err(e) => format!("refused ({e})"),
        },
        t0.elapsed()
    );
    let t0 = Instant::now();
    let served = loop {
        if let Ok((200, _)) =
            connect(port, config).and_then(|mut s| request(&mut s, "/api/v1/info", Some(TOKEN)))
        {
            break t0.elapsed();
        }
        assert!(
            t0.elapsed() <= Duration::from_secs(15),
            "never served again"
        );
        thread::sleep(Duration::from_millis(100));
    };
    println!("stall: a new connection served again after {served:?} (handshake timeout is 5 s)");
    drop(held);
}

/// A subscriber that stops reading while events flow: the server must
/// close it with `resync_required` once its queue fills, and a healthy
/// client must stay answered meanwhile.
fn overflow(port: u16, config: &Arc<ClientConfig>) {
    let s = connect(port, config).unwrap();
    let mut req = "wss://localhost/api/v1/events"
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Authorization", TOKEN.parse().unwrap());
    let (mut ws, response) = tungstenite::client::client(req, s).unwrap();
    assert_eq!(response.status(), 101);
    let hello = ws.read().unwrap();
    println!("overflow: subscribed, first frame {hello}");
    // Stop reading. A healthy client keeps asking meanwhile.
    let healthy_port = port;
    let healthy_config = Arc::clone(config);
    let healthy = thread::spawn(move || {
        let mut s = connect(healthy_port, &healthy_config).unwrap();
        let mut samples = Vec::new();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(12) {
            let t = Instant::now();
            let (status, _) = request(&mut s, "/api/v1/info", Some(TOKEN)).unwrap();
            assert_eq!(status, 200);
            samples.push(t.elapsed());
            thread::sleep(Duration::from_millis(50));
        }
        samples
    });
    let t0 = Instant::now();
    thread::sleep(Duration::from_secs(12));
    // Now drain: the socket holds what the kernel buffered, then the
    // server's queue, then the resync frame and the close.
    let mut frames = 0usize;
    let mut resync = false;
    ws.get_mut()
        .sock
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                frames += 1;
                if t.contains("resync_required") {
                    resync = true;
                }
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    println!(
        "overflow: after {:?} unread, the drain gave {frames} frames, resync_required={resync}, then the close",
        t0.elapsed()
    );
    report(
        "info from the healthy client during the overflow",
        healthy.join().unwrap(),
    );
}

/// The wrong pin: the handshake fails before any request.
fn wrong_pin(port: u16) {
    let config = client_config([0u8; 32]);
    match connect(port, &config) {
        Ok(_) => println!("pin: WRONG, a connection with a bad pin succeeded"),
        Err(e) => println!("pin: a substituted pin is refused at the handshake: {e}"),
    }
}

fn main() {
    let a = args();
    let hex = std::fs::read_to_string(&a.pin_file).unwrap();
    let config = client_config(pin_from_hex(hex.trim()));
    let all = a.phase == "all";
    if all || a.phase == "latency" {
        latency(a.port, &config);
    }
    if all || a.phase == "pin" {
        wrong_pin(a.port);
    }
    if all || a.phase == "overflow" {
        overflow(a.port, &config);
    }
    if all || a.phase == "stall" {
        stall(a.port, &config);
    }
}
