//! The server half of the spike: TLS over `std::net`, one thread per
//! connection, every request handed to one "frame" thread over a
//! bounded channel and answered there, WebSocket subscribers fed from
//! the frame with a bounded queue each, and the connection pools of
//! docs/remote-client.md §4.1.
//!
//! Run: `cargo run --release -- [--port 43187] [--frame-ms 16]
//! [--event-bytes 4096] [--pin-file pin.txt] [--bonjour]`.
//! The frame prints its per-pass cost to stderr every five seconds.

use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use spike_remote_port::{fingerprint, provider, read_request, respond, respond_upgrade};
use tungstenite::Message;
use tungstenite::protocol::Role;

/// §4.1: at most 16 connections, 4 of them unauthenticated.
const MAX_UNAUTH: usize = 4;
const MAX_PAIRED: usize = 12;
/// §4.1: the handshake and the first request line within 5 s, a write
/// within 10 s.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// §4.1: the frame drains at most 8 requests per pass from a channel
/// of 64; each subscriber queues 256 events.
const DRAIN_PER_PASS: usize = 8;
const INBOX: usize = 64;
const SUBSCRIBER_QUEUE: usize = 256;
/// The spike's one token: a paired device presents it as a bearer.
const TOKEN: &str = "Bearer spike-token";

enum ToFrame {
    Info { reply: SyncSender<String> },
    Subscribe(Subscriber),
}

struct Subscriber {
    tx: SyncSender<String>,
    overflow: Arc<AtomicBool>,
}

#[derive(Default)]
struct Pools {
    unauth: AtomicUsize,
    paired: AtomicUsize,
    refused: AtomicUsize,
}

struct Args {
    port: u16,
    frame_ms: u64,
    event_bytes: usize,
    pin_file: String,
    bonjour: bool,
}

fn args() -> Args {
    let mut a = Args {
        port: 43187,
        frame_ms: 16,
        event_bytes: 4096,
        pin_file: "pin.txt".into(),
        bonjour: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--port" => a.port = it.next().unwrap().parse().unwrap(),
            "--frame-ms" => a.frame_ms = it.next().unwrap().parse().unwrap(),
            "--event-bytes" => a.event_bytes = it.next().unwrap().parse().unwrap(),
            "--pin-file" => a.pin_file = it.next().unwrap(),
            "--bonjour" => a.bonjour = true,
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

fn main() {
    let a = args();
    let ck = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let cert: CertificateDer<'static> = ck.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
    let hex = fingerprint(&cert).iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    });
    std::fs::write(&a.pin_file, &hex).unwrap();
    println!("fingerprint {hex}");
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    let config = Arc::new(config);

    let (tx, rx) = mpsc::sync_channel::<ToFrame>(INBOX);
    let frame_ms = a.frame_ms;
    let event_bytes = a.event_bytes;
    thread::spawn(move || frame(&rx, frame_ms, event_bytes));

    let _bonjour = a.bonjour.then(|| advertise(a.port));

    let listener = TcpListener::bind(("127.0.0.1", a.port)).unwrap();
    println!("listening on 127.0.0.1:{}", a.port);
    let pools = Arc::new(Pools::default());
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        // A further accept past the pool is closed at once: dropping
        // the stream is the close.
        if pools.unauth.load(Ordering::SeqCst) >= MAX_UNAUTH {
            pools.refused.fetch_add(1, Ordering::SeqCst);
            drop(stream);
            continue;
        }
        pools.unauth.fetch_add(1, Ordering::SeqCst);
        let config = Arc::clone(&config);
        let tx = tx.clone();
        let pools = Arc::clone(&pools);
        thread::spawn(move || {
            let paired = Arc::new(AtomicBool::new(false));
            if let Err(e) = connection(stream, &config, &tx, &pools, &paired) {
                eprintln!("connection: {e}");
            }
            if paired.load(Ordering::SeqCst) {
                pools.paired.fetch_sub(1, Ordering::SeqCst);
            } else {
                pools.unauth.fetch_sub(1, Ordering::SeqCst);
            }
        });
    }
}

/// Bonjour through `mdns-sd`: the service is registered while the
/// daemon lives, so the daemon is returned to be held.
fn advertise(port: u16) -> mdns_sd::ServiceDaemon {
    let daemon = mdns_sd::ServiceDaemon::new().unwrap();
    let host = format!("{}.local.", hostname());
    let info = mdns_sd::ServiceInfo::new(
        "_switchboard._tcp.local.",
        "Spike 22",
        &host,
        "",
        port,
        &[("v", "1"), ("name", "Spike 22")][..],
    )
    .unwrap()
    .enable_addr_auto();
    daemon.register(info).unwrap();
    println!("advertising _switchboard._tcp as \"Spike 22\" on {host}");
    daemon
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "spike22".into(), |s| s.trim().to_owned())
}

/// The stand-in for the app's frame: a pass every `frame_ms` that
/// drains at most `DRAIN_PER_PASS` requests, bumps the revision, and
/// gives every subscriber one event. A subscriber whose queue is full
/// is dropped with its overflow flag set, never trimmed.
fn frame(rx: &Receiver<ToFrame>, frame_ms: u64, event_bytes: usize) {
    let mut revision: u64 = 0;
    let mut subscribers: Vec<Subscriber> = Vec::new();
    let mut passes: u64 = 0;
    let mut drained: u64 = 0;
    let mut max_us: u128 = 0;
    let mut total_us: u128 = 0;
    let mut last_report = Instant::now();
    let pad = "x".repeat(event_bytes);
    loop {
        thread::sleep(Duration::from_millis(frame_ms));
        let t0 = Instant::now();
        revision += 1;
        for _ in 0..DRAIN_PER_PASS {
            match rx.try_recv() {
                Ok(ToFrame::Info { reply }) => {
                    drained += 1;
                    let _ =
                        reply.try_send(format!("{{\"revision\":{revision},\"pass\":{passes}}}"));
                }
                Ok(ToFrame::Subscribe(s)) => {
                    let _ =
                        s.tx.try_send(format!("{{\"type\":\"hello\",\"revision\":{revision}}}"));
                    subscribers.push(s);
                }
                Err(_) => break,
            }
        }
        let event = format!("{{\"type\":\"changed\",\"revision\":{revision},\"pad\":\"{pad}\"}}");
        subscribers.retain(|s| match s.tx.try_send(event.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                s.overflow.store(true, Ordering::SeqCst);
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        });
        let us = t0.elapsed().as_micros();
        passes += 1;
        total_us += us;
        max_us = max_us.max(us);
        if last_report.elapsed() >= Duration::from_secs(5) {
            eprintln!(
                "frame: passes={passes} drained={drained} subscribers={} max_pass_us={max_us} avg_pass_us={}",
                subscribers.len(),
                total_us / u128::from(passes.max(1))
            );
            last_report = Instant::now();
        }
    }
}

fn connection(
    tcp: TcpStream,
    config: &Arc<ServerConfig>,
    tx: &SyncSender<ToFrame>,
    pools: &Pools,
    paired: &Arc<AtomicBool>,
) -> std::io::Result<()> {
    tcp.set_read_timeout(Some(READ_TIMEOUT))?;
    tcp.set_write_timeout(Some(WRITE_TIMEOUT))?;
    // A reply is one record and goes out at once; Nagle with the
    // client's delayed ACK otherwise adds milliseconds per request.
    tcp.set_nodelay(true)?;
    let conn = ServerConnection::new(Arc::clone(config)).map_err(std::io::Error::other)?;
    let mut stream = StreamOwned::new(conn, tcp);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    let mut reader = BufReader::new(stream);
    while let Some(req) = read_request(&mut reader)? {
        let authed = req.header("authorization") == Some(TOKEN);
        if authed && !paired.load(Ordering::SeqCst) {
            // The device moves from the unauthenticated pool to the
            // paired one on its first authenticated request.
            if pools.paired.load(Ordering::SeqCst) >= MAX_PAIRED {
                respond(
                    reader.get_mut(),
                    503,
                    "Service Unavailable",
                    b"{\"error\":\"full\"}",
                )?;
                return Ok(());
            }
            pools.paired.fetch_add(1, Ordering::SeqCst);
            pools.unauth.fetch_sub(1, Ordering::SeqCst);
            paired.store(true, Ordering::SeqCst);
        }
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/api/v1/info") => {
                let (reply_tx, reply_rx) = mpsc::sync_channel(1);
                match tx.try_send(ToFrame::Info { reply: reply_tx }) {
                    Ok(()) => match reply_rx.recv_timeout(Duration::from_secs(5)) {
                        Ok(body) => respond(reader.get_mut(), 200, "OK", body.as_bytes())?,
                        Err(_) => respond(reader.get_mut(), 504, "Gateway Timeout", b"{}")?,
                    },
                    Err(_) => respond(reader.get_mut(), 503, "Service Unavailable", b"{}")?,
                }
            }
            ("GET", "/api/v1/events") => {
                if !authed {
                    respond(reader.get_mut(), 401, "Unauthorized", b"{}")?;
                    continue;
                }
                let Some(key) = req.header("sec-websocket-key").map(str::to_owned) else {
                    respond(reader.get_mut(), 400, "Bad Request", b"{}")?;
                    continue;
                };
                respond_upgrade(reader.get_mut(), &key)?;
                let stream = reader.into_inner();
                return subscriber(stream, tx);
            }
            _ => respond(reader.get_mut(), 404, "Not Found", b"{}")?,
        }
    }
    Ok(())
}

/// Feed one WebSocket from its queue until the frame drops the queue:
/// on overflow the last frame is `resync_required`, then the close.
fn subscriber(
    stream: StreamOwned<ServerConnection, TcpStream>,
    tx: &SyncSender<ToFrame>,
) -> std::io::Result<()> {
    let mut ws = tungstenite::WebSocket::from_raw_socket(stream, Role::Server, None);
    let (q_tx, q_rx) = mpsc::sync_channel::<String>(SUBSCRIBER_QUEUE);
    let overflow = Arc::new(AtomicBool::new(false));
    tx.try_send(ToFrame::Subscribe(Subscriber {
        tx: q_tx,
        overflow: Arc::clone(&overflow),
    }))
    .map_err(|_| std::io::Error::other("frame inbox full"))?;
    loop {
        let Ok(text) = q_rx.recv() else {
            if overflow.load(Ordering::SeqCst) {
                let _ = ws.send(Message::Text("{\"type\":\"resync_required\"}".into()));
            }
            let _ = ws.close(None);
            let _ = ws.flush();
            return Ok(());
        };
        if let Err(e) = ws.send(Message::Text(text.into())) {
            eprintln!("subscriber write: {e}");
            return Ok(());
        }
    }
}
