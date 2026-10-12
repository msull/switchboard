//! The remote port's adapters: the identity and device files on disk,
//! the host's certificate and key, and the TLS listener that carries
//! requests from paired devices to the frame.
//!
//! The listener has the control socket's shape: one accept thread, one
//! thread per connection, every request handed to the frame over a
//! bounded channel and answered there. What the frame decides (who the
//! bearer is, what the path means, what leaves redacted) lives in
//! `app.rs`; this file only moves bytes and enforces the bounds that
//! need no state: pools, deadlines and rate limits.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use base64::Engine as _;
use rcgen::PublicKeyData as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, DigitallySignedStruct, ServerConfig, ServerConnection, SignatureScheme,
    StreamOwned,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::adapters::store::JsonStore;
use crate::ports::remote::{
    DEVICES_FILE, IDENTITY_FILE, IDENTITY_SCHEMA_VERSION, Identity, Listen, PairedDevice,
    RemoteStore,
};

// --- the files

/// `identity.json` and `devices.json` in the data directory, written
/// the way every store file is: a temp file, fsync, rename.
pub struct RemoteFiles {
    dir: PathBuf,
}

impl RemoteFiles {
    #[must_use]
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn load<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<Option<T>, String> {
        let path = self.dir.join(name);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn save<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
        JsonStore::write_atomic(&self.dir, &self.dir.join(name), &json, false)
            .map_err(|e| e.to_string())
    }
}

impl RemoteStore for RemoteFiles {
    fn load_identity(&self) -> Result<Option<Identity>, String> {
        self.load(IDENTITY_FILE)
    }
    fn save_identity(&self, identity: &Identity) -> Result<(), String> {
        self.save(IDENTITY_FILE, identity)
    }
    fn load_devices(&self) -> Result<Vec<PairedDevice>, String> {
        Ok(self.load(DEVICES_FILE)?.unwrap_or_default())
    }
    fn save_devices(&self, devices: &[PairedDevice]) -> Result<(), String> {
        self.save(DEVICES_FILE, &devices)
    }
}

// --- identity, certificate and key

/// A fresh identity: a P-256 key and a self-signed certificate over it
/// that never expires in practice (rcgen's default `not_after` is the
/// year 4096). Returns the identity and the key as PEM, which goes to
/// the Keychain and nowhere else. `server_id` is kept when given: a
/// rotated key is a new identity for clients but the same installation.
pub fn make_identity(
    server_id: Option<Uuid>,
    server_name: &str,
    listen: Listen,
) -> Result<(Identity, String), String> {
    let server_id = server_id.unwrap_or_else(Uuid::new_v4);
    let key = rcgen::KeyPair::generate().map_err(|e| format!("key: {e}"))?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())
        .map_err(|e| format!("certificate: {e}"))?;
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        format!("Switchboard {server_id}"),
    );
    let cert = params
        .self_signed(&key)
        .map_err(|e| format!("certificate: {e}"))?;
    let der = cert.der().to_vec();
    let spki_sha256 = sha256_hex(&key.subject_public_key_info());
    debug_assert_eq!(
        spki_sha256_of_cert(&der).as_deref(),
        Some(spki_sha256.as_str())
    );
    let identity = Identity {
        schema_version: IDENTITY_SCHEMA_VERSION,
        server_id,
        server_name: server_name.to_owned(),
        certificate: base64::engine::general_purpose::STANDARD.encode(&der),
        spki_sha256,
        created: SystemTime::now(),
        listen,
    };
    Ok((identity, key.serialize_pem()))
}

/// The certificate's DER back from the identity.
pub fn certificate_der(identity: &Identity) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(&identity.certificate)
        .map_err(|e| format!("certificate: {e}"))
}

/// The SPKI hash of a PEM key, to check that the key in the Keychain
/// is the one the identity's certificate was made over.
pub fn spki_sha256_of_key(pem: &str) -> Result<String, String> {
    let key = rcgen::KeyPair::from_pem(pem).map_err(|e| format!("key: {e}"))?;
    Ok(sha256_hex(&key.subject_public_key_info()))
}

/// The server side of TLS: the identity's certificate over the key.
pub fn tls_config(identity: &Identity, key_pem: &str) -> Result<Arc<ServerConfig>, String> {
    let cert = CertificateDer::from(certificate_der(identity)?);
    let key = rcgen::KeyPair::from_pem(key_pem).map_err(|e| format!("key: {e}"))?;
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
    let config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(config))
}

/// The crypto provider every connection uses: `ring`, which the tree
/// already builds. Named explicitly so a second provider in the tree
/// can never make the default ambiguous.
fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A client that trusts exactly one public key, by the hash of its
/// `SubjectPublicKeyInfo`, and nothing else: no names, no dates, no
/// chain. What a paired device does on every connection; here for the
/// tests and for a Rust client.
pub fn pinned_client(spki_sha256: &str) -> Result<Arc<ClientConfig>, String> {
    let provider = provider();
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pin {
            spki_sha256: spki_sha256.to_owned(),
            provider,
        }))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[derive(Debug)]
struct Pin {
    spki_sha256: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pin {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if spki_sha256_of_cert(end_entity).as_deref() == Some(self.spki_sha256.as_str()) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "the host's key is not the pinned one".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// SHA-256 of the certificate's `SubjectPublicKeyInfo`, hex. The
/// certificate is walked as DER: `Certificate { tbsCertificate {
/// [0] version, serialNumber, signature, issuer, validity, subject,
/// subjectPublicKeyInfo, .. }, .. }`, and the whole SPKI element is
/// hashed, which is what `rcgen::KeyPair::public_key_der` returns.
#[must_use]
pub fn spki_sha256_of_cert(der: &[u8]) -> Option<String> {
    let (0x30, tbs_and_rest, _) = der_element(der)? else {
        return None;
    };
    let (0x30, mut rest, _) = der_element(tbs_and_rest)? else {
        return None;
    };
    // The version is explicit and optional; rcgen writes it.
    if rest.first() == Some(&0xA0) {
        rest = der_element(rest)?.2;
    }
    for _ in 0..5 {
        rest = der_element(rest)?.2;
    }
    let (0x30, _, after) = der_element(rest)? else {
        return None;
    };
    Some(sha256_hex(&rest[..rest.len() - after.len()]))
}

/// One DER element: its tag, its content, and what follows it.
fn der_element(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, after_tag) = bytes.split_first()?;
    let (&first, after_first) = after_tag.split_first()?;
    let (length, content_at) = if first < 0x80 {
        (usize::from(first), after_first)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || after_first.len() < count {
            return None;
        }
        let length = after_first[..count]
            .iter()
            .fold(0usize, |n, &b| (n << 8) | usize::from(b));
        (length, &after_first[count..])
    };
    if content_at.len() < length {
        return None;
    }
    let (content, rest) = content_at.split_at(length);
    Some((tag, content, rest))
}

/// Lowercase hex of SHA-256 over `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// The name a new identity starts with: the machine's short host name,
/// or "Switchboard" when it has none.
#[must_use]
pub fn default_server_name() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Switchboard".into())
}

// --- random material

/// The pairing code's alphabet: Crockford's base 32, with no I, L, O
/// or U, so a code read aloud or typed is never ambiguous.
pub const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Eight characters of five bits: 40 bits, single use, three tries.
pub const CODE_LENGTH: usize = 8;

/// A fresh pairing code from the OS random source.
#[must_use]
pub fn pairing_code() -> String {
    let mut bytes = [0u8; CODE_LENGTH];
    fill_random(&mut bytes);
    bytes
        .iter()
        .map(|b| char::from(CODE_ALPHABET[usize::from(b % 32)]))
        .collect()
}

/// A fresh bearer token: 32 random bytes as base64url without padding,
/// 43 characters. It is shown to the device once and never stored.
#[must_use]
pub fn bearer_token() -> String {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// What `devices.json` keeps of a token.
#[must_use]
pub fn token_sha256(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

fn fill_random(bytes: &mut [u8]) {
    // The OS random source failing is not something to carry on from.
    getrandom::fill(bytes).expect("the OS random source");
}

// --- HTTP

/// One request as it came off the socket, with what the frame needs
/// and nothing it does not: the bearer token is taken out of the
/// headers so it is never logged with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRequest {
    pub method: String,
    /// The path without its query string.
    pub path: String,
    /// The query string's pairs, undecoded: ids and numbers only.
    pub query: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The `Authorization: Bearer` value, if any.
    pub bearer: Option<String>,
}

impl RemoteRequest {
    /// The first value of a query parameter.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// What the frame answers: a status and a JSON body, and whether the
/// request was made by a paired device, which moves the connection
/// from the unauthenticated pool to the paired one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteReply {
    pub status: u16,
    pub body: Vec<u8>,
    pub authenticated: bool,
}

/// One request off a connection, with the channel its reply goes back on.
pub struct RemoteIncoming {
    pub request: RemoteRequest,
    pub reply: SyncSender<RemoteReply>,
}

/// The most a request line and its headers may take, and the most a
/// body may be.
pub const MAX_HEAD: usize = 8 * 1024;
pub const MAX_BODY: usize = 64 * 1024;

/// The head of one HTTP/1.1 request: method, target, lowercased
/// headers, body. `None` at a clean end of stream before any byte; an
/// oversized or malformed request is an error that closes the
/// connection.
fn read_request<R: Read>(reader: &mut BufReader<R>) -> io::Result<Option<RemoteRequest>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad request line",
        ));
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut total = line.len();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "headers cut"));
        }
        total += h.len();
        if total > MAX_HEAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large"));
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((n, v)) = h.split_once(':') {
            headers.push((n.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    let length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
    }
    let bearer = header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let (n, v) = pair.split_once('=').unwrap_or((pair, ""));
            (n.to_owned(), v.to_owned())
        })
        .collect();
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(RemoteRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        query,
        body,
        bearer,
    }))
}

/// Write one response. Head and body in one write, so a reply is one
/// TLS record and one segment.
fn respond<W: Write>(w: &mut W, status: u16, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    w.write_all(&out)?;
    w.flush()
}

/// The error body the connection thread sends for what never reaches
/// the frame.
fn error_body(code: &str, message: &str) -> Vec<u8> {
    serde_json::json!({ "error": { "code": code, "message": message } })
        .to_string()
        .into_bytes()
}

// --- the listener

/// At most 16 connections, 4 of them unauthenticated. The 4 are a pool
/// of their own: a flood of strangers fills its own slots and never a
/// paired device's.
pub const MAX_UNAUTHENTICATED: usize = 4;
pub const MAX_PAIRED: usize = 12;
/// The TLS handshake and the first request line within this.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// A paired device polling on a kept connection may sit idle this long
/// between requests.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a connection waits for the frame's answer. The frame
/// answers on its next pass, so this only matters if it has stalled.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
/// The frame drains at most this many requests per pass from a channel
/// this deep; a full channel answers 503 without reaching the frame.
pub const DRAIN_PER_PASS: usize = 8;
const INBOX: usize = 64;
/// `/pair` and `/info` per minute per listener, answered 429 past it.
const PAIR_PER_MINUTE: usize = 6;
const INFO_PER_MINUTE: usize = 60;

const PAIR_PATH: &str = "/api/v1/pair";
const INFO_PATH: &str = "/api/v1/info";

#[derive(Default)]
struct Pools {
    unauthenticated: AtomicUsize,
    paired: AtomicUsize,
}

/// Allows `max` events per `per`, counted on a sliding window.
struct RateWindow {
    stamps: VecDeque<Instant>,
    max: usize,
    per: Duration,
}

impl RateWindow {
    fn new(max: usize, per: Duration) -> Self {
        Self {
            stamps: VecDeque::new(),
            max,
            per,
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        while self.stamps.front().is_some_and(|&t| now - t >= self.per) {
            self.stamps.pop_front();
        }
        if self.stamps.len() >= self.max {
            return false;
        }
        self.stamps.push_back(now);
        true
    }
}

struct Limits {
    pair: Mutex<RateWindow>,
    info: Mutex<RateWindow>,
}

/// The TLS listener: an accept thread, one thread per connection,
/// requests handed to the frame over a bounded channel. Bound only by
/// the instance that holds the store lock.
pub struct RemotePort {
    addr: SocketAddr,
    requests: Receiver<RemoteIncoming>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RemotePort {
    /// Bind `addr` (port 0 picks one) with the host's TLS configuration.
    /// `wake` runs on a connection thread whenever a request is queued
    /// (an egui `request_repaint`).
    pub fn bind(
        addr: SocketAddr,
        tls: Arc<ServerConfig>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let (tx, requests) = mpsc::sync_channel(INBOX);
        let stop = Arc::new(AtomicBool::new(false));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let pools = Arc::new(Pools::default());
        let limits = Arc::new(Limits {
            pair: Mutex::new(RateWindow::new(PAIR_PER_MINUTE, Duration::from_secs(60))),
            info: Mutex::new(RateWindow::new(INFO_PER_MINUTE, Duration::from_secs(60))),
        });
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("switchboard-remote".into())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(stream) = stream else { continue };
                        // Past the pool a stranger is closed at once:
                        // dropping the stream is the close.
                        if pools.unauthenticated.load(Ordering::SeqCst) >= MAX_UNAUTHENTICATED {
                            drop(stream);
                            continue;
                        }
                        pools.unauthenticated.fetch_add(1, Ordering::SeqCst);
                        let tls = Arc::clone(&tls);
                        let tx = tx.clone();
                        let wake = Arc::clone(&wake);
                        let pools = Arc::clone(&pools);
                        let limits = Arc::clone(&limits);
                        let _ = std::thread::Builder::new()
                            .name("switchboard-remote-conn".into())
                            .spawn(move || {
                                let paired = AtomicBool::new(false);
                                if let Err(e) =
                                    connection(stream, &tls, &tx, &*wake, &pools, &limits, &paired)
                                {
                                    log::debug!("remote connection: {e}");
                                }
                                if paired.load(Ordering::SeqCst) {
                                    pools.paired.fetch_sub(1, Ordering::SeqCst);
                                } else {
                                    pools.unauthenticated.fetch_sub(1, Ordering::SeqCst);
                                }
                            });
                    }
                })?
        };
        Ok(Self {
            addr,
            requests,
            stop,
            thread: Some(thread),
        })
    }

    /// Up to `max` requests that arrived since the last poll, in
    /// arrival order; the rest wait on the channel.
    pub fn poll(&mut self, max: usize) -> Vec<RemoteIncoming> {
        self.requests.try_iter().take(max).collect()
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for RemotePort {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accept loop only checks the flag after a connection, so
        // make one to unblock it.
        let unblock = if self.addr.ip().is_unspecified() {
            SocketAddr::new(IpAddr::from([127, 0, 0, 1]), self.addr.port())
        } else {
            self.addr
        };
        let _ = TcpStream::connect_timeout(&unblock, Duration::from_secs(1));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One connection: the handshake, then a request and a reply at a time
/// until the device hangs up or a deadline passes.
fn connection(
    tcp: TcpStream,
    tls: &Arc<ServerConfig>,
    tx: &SyncSender<RemoteIncoming>,
    wake: &dyn Fn(),
    pools: &Pools,
    limits: &Limits,
    paired: &AtomicBool,
) -> io::Result<()> {
    tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    tcp.set_write_timeout(Some(WRITE_TIMEOUT))?;
    // A reply is one record and goes out at once; Nagle with the
    // device's delayed ACK would otherwise add milliseconds per request.
    tcp.set_nodelay(true)?;
    let conn = ServerConnection::new(Arc::clone(tls)).map_err(io::Error::other)?;
    let mut stream = StreamOwned::new(conn, tcp);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    let mut reader = BufReader::new(stream);
    let mut first = true;
    while let Some(request) = read_request(&mut reader)? {
        if first {
            reader.get_ref().sock.set_read_timeout(Some(IDLE_TIMEOUT))?;
            first = false;
        }
        let limited = match request.path.as_str() {
            PAIR_PATH => !limits
                .pair
                .lock()
                .map_or(true, |mut w| w.allow(Instant::now())),
            INFO_PATH => !limits
                .info
                .lock()
                .map_or(true, |mut w| w.allow(Instant::now())),
            _ => false,
        };
        if limited {
            respond(
                reader.get_mut(),
                429,
                &error_body("rate_limited", "too many requests; try again in a minute"),
            )?;
            continue;
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        if tx
            .try_send(RemoteIncoming {
                request,
                reply: reply_tx,
            })
            .is_err()
        {
            respond(
                reader.get_mut(),
                503,
                &error_body("busy", "the host is busy; try again"),
            )?;
            continue;
        }
        wake();
        let reply = match reply_rx.recv_timeout(ANSWER_TIMEOUT) {
            Ok(reply) => reply,
            Err(_) => RemoteReply {
                status: 504,
                body: error_body("no_answer", "the app did not answer in time"),
                authenticated: false,
            },
        };
        if reply.authenticated && !paired.load(Ordering::SeqCst) {
            // The device moves from the unauthenticated pool to the
            // paired one on its first authenticated request.
            if pools.paired.load(Ordering::SeqCst) >= MAX_PAIRED {
                respond(
                    reader.get_mut(),
                    503,
                    &error_body("full", "too many paired connections"),
                )?;
                return Ok(());
            }
            pools.paired.fetch_add(1, Ordering::SeqCst);
            pools.unauthenticated.fetch_sub(1, Ordering::SeqCst);
            paired.store(true, Ordering::SeqCst);
        }
        respond(reader.get_mut(), reply.status, &reply.body)?;
    }
    Ok(())
}

/// The address a device on the local network would reach this machine
/// at: the one the system routes outward on. No packet is sent; a
/// connected UDP socket only chooses a source address. `None` with no
/// network.
#[must_use]
pub fn local_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    socket
        .local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_round_trips_and_its_key_hashes_to_its_spki() {
        let dir = tempfile::tempdir().unwrap();
        let files = RemoteFiles::new(dir.path().to_path_buf());
        assert_eq!(files.load_identity().unwrap(), None);
        assert!(files.load_devices().unwrap().is_empty());
        let (identity, key) = make_identity(None, "Sully's Mac", Listen::default()).unwrap();
        assert_eq!(spki_sha256_of_key(&key).unwrap(), identity.spki_sha256);
        let der = certificate_der(&identity).unwrap();
        assert_eq!(
            spki_sha256_of_cert(&der).as_deref(),
            Some(identity.spki_sha256.as_str())
        );
        files.save_identity(&identity).unwrap();
        assert_eq!(files.load_identity().unwrap(), Some(identity.clone()));
        let device = PairedDevice {
            id: Uuid::new_v4(),
            name: "Vision Pro".into(),
            scope: crate::ports::remote::Scope::Read,
            token_sha256: token_sha256("t"),
            created: SystemTime::UNIX_EPOCH,
            last_seen: None,
        };
        files.save_devices(std::slice::from_ref(&device)).unwrap();
        assert_eq!(files.load_devices().unwrap(), vec![device]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(DEVICES_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        // The same installation under a rotated key.
        let (rotated, _) = make_identity(
            Some(identity.server_id),
            &identity.server_name,
            Listen::default(),
        )
        .unwrap();
        assert_eq!(rotated.server_id, identity.server_id);
        assert_ne!(rotated.spki_sha256, identity.spki_sha256);
        assert!(tls_config(&identity, &key).is_ok());
    }

    #[test]
    fn a_malformed_file_is_an_error_not_a_fresh_start() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(IDENTITY_FILE), b"{not json").unwrap();
        let files = RemoteFiles::new(dir.path().to_path_buf());
        assert!(files.load_identity().is_err());
    }

    #[test]
    fn codes_and_tokens_have_their_shapes() {
        let code = pairing_code();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        assert!(!code.contains(['I', 'L', 'O', 'U']));
        let token = bearer_token();
        assert_eq!(token.len(), 43);
        assert!(!token.contains(['=', '+', '/']));
        assert_ne!(pairing_code(), code);
        assert_eq!(token_sha256("abc").len(), 64);
    }

    #[test]
    fn a_request_reads_with_its_query_bearer_and_body() {
        let text = b"POST /api/v1/x?lines=5&a=b HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer tok\r\nContent-Length: 2\r\n\r\nhi";
        let mut r = BufReader::new(&text[..]);
        let req = read_request(&mut r).unwrap().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/api/v1/x");
        assert_eq!(req.param("lines"), Some("5"));
        assert_eq!(req.bearer.as_deref(), Some("tok"));
        assert_eq!(req.body, b"hi");
        assert!(read_request(&mut r).unwrap().is_none());
        let text = b"POST /x HTTP/1.1\r\nContent-Length: 100000\r\n\r\n";
        let mut r = BufReader::new(&text[..]);
        assert!(
            read_request(&mut r).is_err(),
            "an oversized body is refused before it is read"
        );
    }

    #[test]
    fn the_rate_window_slides() {
        let t0 = Instant::now();
        let mut w = RateWindow::new(2, Duration::from_secs(60));
        assert!(w.allow(t0));
        assert!(w.allow(t0 + Duration::from_secs(1)));
        assert!(!w.allow(t0 + Duration::from_secs(2)));
        assert!(w.allow(t0 + Duration::from_secs(61)));
    }

    #[test]
    fn der_elements_walk_short_and_long_lengths() {
        let short = [0x30, 0x02, 0x05, 0x00, 0xff];
        let (tag, content, rest) = der_element(&short).unwrap();
        assert_eq!((tag, content, rest), (0x30, &short[2..4], &short[4..]));
        let long = [0x04, 0x82, 0x01, 0x00]
            .into_iter()
            .chain(std::iter::repeat_n(7u8, 256))
            .collect::<Vec<_>>();
        let (_, content, rest) = der_element(&long).unwrap();
        assert_eq!(content.len(), 256);
        assert!(rest.is_empty());
        assert!(der_element(&[0x30, 0x05, 0x00]).is_none(), "a cut element");
        assert!(spki_sha256_of_cert(b"not a certificate").is_none());
    }
}
