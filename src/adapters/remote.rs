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

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
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
use crate::adapters::web;
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
    /// The `Sec-WebSocket-Key` of a request that asks to upgrade to a
    /// WebSocket; the frame answers [`SUBSCRIBED`] to accept it.
    pub upgrade: Option<String>,
    /// The `Cookie` header as sent: a browser's credential travels in
    /// it, so it is kept out of the headers for the same reason.
    pub cookie: Option<String>,
    /// The `Origin` header, which a browser sends and a native client
    /// does not.
    pub origin: Option<String>,
    /// The `Host` header: the address the device used to reach this
    /// port, which a same-origin page's `Origin` must name.
    pub host: Option<String>,
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

    /// The value of the cookie with exactly this name, if the request
    /// carries one; every other cookie is ignored.
    #[must_use]
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookie
            .as_deref()?
            .split(';')
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.trim())
            .filter(|v| !v.is_empty())
    }
}

/// What the frame answers: a status and a JSON body, whether the
/// request was made by a paired device, which moves the connection
/// from the unauthenticated pool to the paired one, and any headers
/// beyond the ones every reply carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteReply {
    pub status: u16,
    pub body: Vec<u8>,
    pub authenticated: bool,
    /// Headers beyond the ones `respond` always writes, such as a
    /// `Set-Cookie` for a browser that paired.
    pub headers: Vec<(&'static str, String)>,
}

impl RemoteReply {
    /// A reply with no extra headers, which is nearly every reply.
    #[must_use]
    pub fn new(status: u16, body: Vec<u8>, authenticated: bool) -> Self {
        Self {
            status,
            body,
            authenticated,
            headers: Vec::new(),
        }
    }
}

/// One request off a connection, with the channel its reply goes back
/// on and, for an upgrade request, the queue the connection will feed
/// its WebSocket from once the frame accepts.
pub struct RemoteIncoming {
    pub request: RemoteRequest,
    pub reply: SyncSender<RemoteReply>,
    pub subscriber: Option<SubscriberQueue>,
}

/// The status the frame answers an upgrade request with to accept it;
/// the connection thread then writes the 101 and becomes a subscriber.
pub const SUBSCRIBED: u16 = 101;

/// How many events a subscriber may fall behind before it is dropped.
pub const SUBSCRIBER_QUEUE: usize = 256;

/// How long a subscriber's thread waits for an event before it looks
/// for a frame from the device (a close, a ping, a `watch`).
const SUBSCRIBER_POLL: Duration = Duration::from_millis(50);

/// The frame's end of a subscriber: events go in as JSON text; a full
/// queue drops the sender with `overflow` set, which the connection
/// thread turns into `resync_required` and a close. `id` names the
/// subscriber in the frames it sends back.
pub struct SubscriberQueue {
    pub id: u64,
    pub tx: SyncSender<String>,
    pub overflow: Arc<AtomicBool>,
}

/// A text frame a subscribed device sent (`watch`, `unwatch`), or
/// `None` once its socket ended, however it ended.
pub struct SubscriberFrame {
    pub subscriber: u64,
    pub text: Option<String>,
}

/// How many device frames may wait for the frame thread.
const FRAMES: usize = 64;

/// The most a request line and its headers may take, and the most a
/// body may be.
pub const MAX_HEAD: usize = 8 * 1024;
pub const MAX_BODY: usize = 64 * 1024;

/// The most a subscriber's inbound WebSocket frame or message may be. A
/// device only sends `watch` and `unwatch` JSON, so anything near this
/// is not a client of ours; the library's defaults (16 MiB a frame)
/// would let one socket hold that much.
const SUBSCRIBER_FRAME_MAX: usize = 16 * 1024;

/// One line of the head, `None` at end of stream. `used` is what the
/// head has taken so far; the read stops one byte past what is left of
/// `MAX_HEAD`, so a line with no newline costs at most that much memory
/// and is refused rather than read until the peer stops.
fn read_head_line<R: BufRead>(reader: &mut R, used: usize) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    let budget = MAX_HEAD.saturating_sub(used) as u64 + 1;
    let n = reader.by_ref().take(budget).read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if used + n > MAX_HEAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large"));
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// One HTTP/1.1 request: its head, then its body. `None` at a clean
/// end of stream before any byte; an oversized or malformed request is
/// an error that closes the connection.
#[cfg(test)]
fn read_request<R: Read>(reader: &mut BufReader<R>) -> io::Result<Option<RemoteRequest>> {
    let Some((mut request, length)) = read_head(reader)? else {
        return Ok(None);
    };
    request.body = read_body(reader, length)?;
    Ok(Some(request))
}

/// The head of one HTTP/1.1 request: method, target, lowercased
/// headers, and the body's declared length, which `read_body` reads.
/// The two are apart so a connection can give the body a deadline of
/// its own once the head is in.
fn read_head<R: Read>(reader: &mut BufReader<R>) -> io::Result<Option<(RemoteRequest, usize)>> {
    let Some(line) = read_head_line(reader, 0)? else {
        return Ok(None);
    };
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
        let Some(h) = read_head_line(reader, total)? else {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "headers cut"));
        };
        total += h.len();
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
    let upgrade = header("upgrade")
        .filter(|u| u.eq_ignore_ascii_case("websocket"))
        .and_then(|_| header("sec-websocket-key"))
        .map(str::to_owned);
    let cookie = header("cookie").map(str::to_owned);
    let origin = header("origin").map(str::to_owned);
    let host = header("host").map(str::to_owned);
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let (n, v) = pair.split_once('=').unwrap_or((pair, ""));
            (n.to_owned(), v.to_owned())
        })
        .collect();
    let request = RemoteRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        query,
        body: Vec::new(),
        bearer,
        upgrade,
        cookie,
        origin,
        host,
    };
    Ok(Some((request, length)))
}

/// The `length` bytes of body that follow a head.
fn read_body<R: Read>(reader: &mut BufReader<R>, length: usize) -> io::Result<Vec<u8>> {
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(body)
}

/// The 101 that switches a connection to WebSocket. The request was
/// already read by `read_request`, so `tungstenite::accept` cannot be
/// used; the stream is wrapped with `from_raw_socket` after this.
fn respond_upgrade<W: Write>(w: &mut W, key: &str) -> io::Result<()> {
    write!(
        w,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        tungstenite::handshake::derive_accept_key(key.as_bytes())
    )?;
    w.flush()
}

/// Write one JSON response, announcing whether the connection stays
/// open after it.
fn respond<W: Write>(
    w: &mut W,
    status: u16,
    body: &[u8],
    headers: &[(&'static str, String)],
    keep_alive: bool,
) -> io::Result<()> {
    // No client reads `Cache-Control` or `nosniff`; they are there so a
    // browser neither keeps a reply nor reads it as anything but JSON.
    let mut lines = String::from("Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n");
    for (name, value) in headers {
        let _ = write!(lines, "{name}: {value}\r\n");
    }
    lines.push_str(if keep_alive {
        "Connection: keep-alive\r\n"
    } else {
        "Connection: close\r\n"
    });
    write_reply(w, status, "application/json", body, &lines)
}

/// Write one JSON response and, on a connection that is not paired,
/// end the TLS session after it: a stranger gets one answer per
/// connection and so never holds a slot between requests. `true` when
/// the connection stays open for another request.
fn answer(
    stream: &mut Tls,
    paired: bool,
    status: u16,
    body: &[u8],
    headers: &[(&'static str, String)],
) -> io::Result<bool> {
    respond(stream, status, body, headers, paired)?;
    if !paired {
        stream.conn.send_close_notify();
        stream.flush()?;
    }
    Ok(paired)
}

/// Write one of the page's files with the headers that hold it to
/// itself, and close: a browser's page load must not keep a stranger's
/// slot for the idle timeout.
fn respond_page<W: Write>(w: &mut W, page: &web::Page) -> io::Result<()> {
    let mut lines = String::new();
    for (name, value) in web::PAGE_HEADERS {
        let _ = write!(lines, "{name}: {value}\r\n");
    }
    lines.push_str("Connection: close\r\n");
    write_reply(w, 200, page.content_type, page.body.as_bytes(), &lines)
}

/// Write one response: the status line, `Content-Type`,
/// `Content-Length`, the header lines given (each ending in CRLF), and
/// the body. Head and body in one write, so a reply is one TLS record
/// and one segment.
fn write_reply<W: Write>(
    w: &mut W,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: &str,
) -> io::Result<()> {
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
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{headers}\r\n",
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

/// The error body for a request over its window's limit.
fn rate_limited() -> Vec<u8> {
    error_body("rate_limited", "too many requests; try again in a minute")
}

// --- the listener

/// At most 16 connections, 4 of them unauthenticated. The 4 are a pool
/// of their own: a flood of strangers fills its own slots and never a
/// paired device's.
pub const MAX_UNAUTHENTICATED: usize = 4;
pub const MAX_PAIRED: usize = 12;
/// How many of the unauthenticated slots one address may hold, so one
/// stranger cannot take all four. A page load opens up to three
/// connections in parallel from one address (spike 23); how many a
/// browser opens is unmeasured, and the owner may lower this once one
/// is measured.
pub const MAX_UNAUTHENTICATED_PER_SOURCE: usize = 3;
/// How long an unpaired connection has to finish the TLS handshake,
/// counted from the accept.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an unpaired connection has to deliver the request head,
/// counted from the end of the handshake.
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an unpaired connection has to deliver the request body,
/// counted from the end of the head.
const BODY_TIMEOUT: Duration = Duration::from_secs(10);
/// A paired device polling on a kept connection may sit idle this long
/// between requests.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a connection waits for the frame's answer. The frame
/// answers on its next pass, so this only matters if it has stalled.
pub(crate) const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
/// The frame drains at most this many requests per pass from a channel
/// this deep; a full channel answers 503 without reaching the frame.
pub const DRAIN_PER_PASS: usize = 8;
const INBOX: usize = 64;
/// `/pair` and `/info` per minute per listener, answered 429 past it.
const PAIR_PER_MINUTE: usize = 6;
const INFO_PER_MINUTE: usize = 60;
/// The page's files per minute per listener: one load is three.
const PAGE_PER_MINUTE: usize = 120;

const PAIR_PATH: &str = "/api/v1/pair";
const INFO_PATH: &str = "/api/v1/info";

/// The deadlines of a connection that is not paired; the tests use
/// short ones.
#[derive(Clone, Copy)]
struct Timeouts {
    handshake: Duration,
    head: Duration,
    body: Duration,
}

impl Timeouts {
    const DEFAULT: Self = Self {
        handshake: HANDSHAKE_TIMEOUT,
        head: HEAD_TIMEOUT,
        body: BODY_TIMEOUT,
    };
}

/// Why a connection was closed at accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The unauthenticated pool is full.
    Total,
    /// Its address holds its share of the pool already.
    Source,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Total => "total",
            Self::Source => "source",
        })
    }
}

/// The unauthenticated pool's count, in all and per peer address. It
/// takes addresses and holds no clock, so it is tested with made-up
/// ones. A source is one address, not a prefix.
struct Admission {
    total: usize,
    by_source: HashMap<IpAddr, usize>,
    max_total: usize,
    max_per_source: usize,
}

impl Admission {
    fn new(max_total: usize, max_per_source: usize) -> Self {
        Self {
            total: 0,
            by_source: HashMap::new(),
            max_total,
            max_per_source,
        }
    }

    /// Count one connection from `ip`, or say why not.
    fn admit(&mut self, ip: IpAddr) -> Result<(), Refusal> {
        // An IPv4-mapped IPv6 address is the same peer as its IPv4 form.
        let ip = ip.to_canonical();
        if self.total >= self.max_total {
            return Err(Refusal::Total);
        }
        let held = self.by_source.entry(ip).or_insert(0);
        if *held >= self.max_per_source {
            return Err(Refusal::Source);
        }
        *held += 1;
        self.total += 1;
        Ok(())
    }

    /// Give back one connection `admit` counted for `ip`.
    fn release(&mut self, ip: IpAddr) {
        let ip = ip.to_canonical();
        if let Some(held) = self.by_source.get_mut(&ip) {
            *held -= 1;
            if *held == 0 {
                self.by_source.remove(&ip);
            }
            self.total = self.total.saturating_sub(1);
        }
    }
}

struct Pools {
    unauthenticated: Mutex<Admission>,
    paired: AtomicUsize,
}

impl Pools {
    fn new() -> Self {
        Self {
            unauthenticated: Mutex::new(Admission::new(
                MAX_UNAUTHENTICATED,
                MAX_UNAUTHENTICATED_PER_SOURCE,
            )),
            paired: AtomicUsize::new(0),
        }
    }

    /// A poisoned lock refuses, never admits.
    fn admit(&self, ip: IpAddr) -> Result<(), Refusal> {
        self.unauthenticated
            .lock()
            .map_or(Err(Refusal::Total), |mut a| a.admit(ip))
    }

    /// A poisoned lock still releases, so no count is stranded.
    fn release(&self, ip: IpAddr) {
        self.unauthenticated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .release(ip);
    }
}

/// One connection's place in a pool, given back when it drops: when
/// the connection ends, when its thread panics, and when the thread is
/// never made (a failed spawn drops the closure that holds it).
struct Slot {
    pools: Arc<Pools>,
    ip: IpAddr,
    paired: bool,
}

impl Slot {
    /// An unauthenticated slot for `ip`, if the pool admits it.
    fn take(pools: &Arc<Pools>, ip: IpAddr) -> Result<Self, Refusal> {
        pools.admit(ip)?;
        Ok(Self {
            pools: Arc::clone(pools),
            ip,
            paired: false,
        })
    }

    fn is_paired(&self) -> bool {
        self.paired
    }

    /// Move from the unauthenticated pool to the paired one.
    fn promote(&mut self) {
        if !self.paired {
            self.pools.paired.fetch_add(1, Ordering::SeqCst);
            self.pools.release(self.ip);
            self.paired = true;
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if self.paired {
            self.pools.paired.fetch_sub(1, Ordering::SeqCst);
        } else {
            self.pools.release(self.ip);
        }
    }
}

/// The socket under a connection's TLS, with the deadline the current
/// phase must end by. The deadline is enforced on every socket read,
/// beneath rustls: one read of plaintext may take many reads of the
/// socket while a record trickles in, so a timeout set once above it
/// would bound each byte and never the whole.
struct Timed {
    sock: TcpStream,
    deadline: Option<Instant>,
}

impl Timed {
    fn into_inner(self) -> TcpStream {
        self.sock
    }
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "deadline passed"));
            }
            self.sock.set_read_timeout(Some(left))?;
        }
        self.sock.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sock.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sock.flush()
    }
}

type Tls = StreamOwned<ServerConnection, Timed>;

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
    page: Mutex<RateWindow>,
}

impl Limits {
    /// Whether a request for `path` is over its window. Only `/pair` and
    /// `/info` have one here; any other path is never over.
    fn limited(&self, path: &str) -> bool {
        match path {
            PAIR_PATH => Self::over(&self.pair),
            INFO_PATH => Self::over(&self.info),
            _ => false,
        }
    }

    /// Count one event against `window`; a poisoned lock counts as
    /// over, never as open.
    fn over(window: &Mutex<RateWindow>) -> bool {
        !window.lock().map_or(true, |mut w| w.allow(Instant::now()))
    }
}

/// What every connection thread shares.
struct Shared {
    tls: Arc<ServerConfig>,
    requests: SyncSender<RemoteIncoming>,
    frames: SyncSender<SubscriberFrame>,
    wake: Arc<dyn Fn() + Send + Sync>,
    pools: Arc<Pools>,
    limits: Limits,
    timeouts: Timeouts,
    next_subscriber: AtomicU64,
}

/// The TLS listener: an accept thread, one thread per connection,
/// requests handed to the frame over a bounded channel. Bound only by
/// the instance that holds the store lock.
pub struct RemotePort {
    addr: SocketAddr,
    requests: Receiver<RemoteIncoming>,
    frames: Receiver<SubscriberFrame>,
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
        Self::bind_with(addr, tls, wake, Timeouts::DEFAULT)
    }

    fn bind_with(
        addr: SocketAddr,
        tls: Arc<ServerConfig>,
        wake: impl Fn() + Send + Sync + 'static,
        timeouts: Timeouts,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let (tx, requests) = mpsc::sync_channel(INBOX);
        let (frames_tx, frames) = mpsc::sync_channel(FRAMES);
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            tls,
            requests: tx,
            frames: frames_tx,
            wake: Arc::new(wake),
            pools: Arc::new(Pools::new()),
            limits: Limits {
                pair: Mutex::new(RateWindow::new(PAIR_PER_MINUTE, Duration::from_secs(60))),
                info: Mutex::new(RateWindow::new(INFO_PER_MINUTE, Duration::from_secs(60))),
                page: Mutex::new(RateWindow::new(PAGE_PER_MINUTE, Duration::from_secs(60))),
            },
            timeouts,
            next_subscriber: AtomicU64::new(1),
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
                        // Past the pool, or past its address's share of
                        // it, a stranger is closed at once: dropping the
                        // stream is the close.
                        let ip = match stream.peer_addr() {
                            Ok(peer) => peer.ip(),
                            Err(e) => {
                                log::debug!("remote: refused a peer with no address: {e}");
                                continue;
                            }
                        };
                        let mut slot = match Slot::take(&shared.pools, ip) {
                            Ok(slot) => slot,
                            Err(refusal) => {
                                log::debug!("remote: refused {ip}: {refusal}");
                                continue;
                            }
                        };
                        let shared = Arc::clone(&shared);
                        // The slot moves into the closure, so a spawn
                        // that fails drops it and frees its place.
                        if let Err(e) = std::thread::Builder::new()
                            .name("switchboard-remote-conn".into())
                            .spawn(move || {
                                if let Err(e) = connection(stream, &shared, &mut slot) {
                                    log::debug!("remote connection: {e}");
                                }
                            })
                        {
                            log::warn!("remote: no thread for {ip}: {e}");
                        }
                    }
                })?
        };
        Ok(Self {
            addr,
            requests,
            frames,
            stop,
            thread: Some(thread),
        })
    }

    /// Up to `max` requests that arrived since the last poll, in
    /// arrival order; the rest wait on the channel.
    pub fn poll(&mut self, max: usize) -> Vec<RemoteIncoming> {
        self.requests.try_iter().take(max).collect()
    }

    /// Every frame subscribed devices sent since the last poll.
    pub fn poll_frames(&mut self) -> Vec<SubscriberFrame> {
        self.frames.try_iter().collect()
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

/// The TLS session over an accepted socket, its handshake done within
/// the handshake deadline.
fn handshake(tcp: TcpStream, shared: &Shared) -> io::Result<Tls> {
    tcp.set_write_timeout(Some(WRITE_TIMEOUT))?;
    // A reply is one record and goes out at once; Nagle with the
    // device's delayed ACK would otherwise add milliseconds per request.
    tcp.set_nodelay(true)?;
    let conn = ServerConnection::new(Arc::clone(&shared.tls)).map_err(io::Error::other)?;
    let sock = Timed {
        sock: tcp,
        deadline: Some(Instant::now() + shared.timeouts.handshake),
    };
    let mut stream = StreamOwned::new(conn, sock);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    Ok(stream)
}

/// The next request, `None` once the device hangs up. On a connection
/// that is not paired the head and then the body each run on a deadline
/// of their own; a paired one reads under its idle timeout alone.
fn next_request(
    reader: &mut BufReader<Tls>,
    paired: bool,
    timeouts: Timeouts,
) -> io::Result<Option<RemoteRequest>> {
    if !paired {
        reader.get_mut().sock.deadline = Some(Instant::now() + timeouts.head);
    }
    let Some((mut request, length)) = read_head(reader)? else {
        return Ok(None);
    };
    if !paired {
        reader.get_mut().sock.deadline = Some(Instant::now() + timeouts.body);
    }
    request.body = read_body(reader, length)?;
    Ok(Some(request))
}

/// One connection: the handshake, then a request and a reply at a time
/// until the device hangs up or a deadline passes. Until it is paired
/// it runs on the absolute deadlines in `shared.timeouts` and is closed
/// after its first reply; once paired it may idle `IDLE_TIMEOUT`
/// between requests.
fn connection(tcp: TcpStream, shared: &Arc<Shared>, slot: &mut Slot) -> io::Result<()> {
    let mut reader = BufReader::new(handshake(tcp, shared)?);
    while let Some(request) = next_request(&mut reader, slot.is_paired(), shared.timeouts)? {
        // The page's files are constant bytes, answered here so a load
        // never waits on a busy frame. A query on them is ignored. They
        // have a rate window of their own; `/pair` and `/info` theirs.
        let page = web::page(&request.method, &request.path);
        let over = match page {
            Some(_) => Limits::over(&shared.limits.page),
            None => shared.limits.limited(&request.path),
        };
        if over {
            if answer(
                reader.get_mut(),
                slot.is_paired(),
                429,
                &rate_limited(),
                &[],
            )? {
                continue;
            }
            return Ok(());
        }
        if let Some(page) = page {
            let stream = reader.get_mut();
            respond_page(stream, page)?;
            stream.conn.send_close_notify();
            return stream.flush();
        }
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let upgrade_key = request.upgrade.clone();
        let mut queue = None;
        let queue_for_frame = upgrade_key.as_ref().map(|_| {
            let (q_tx, q_rx) = mpsc::sync_channel::<String>(SUBSCRIBER_QUEUE);
            let overflow = Arc::new(AtomicBool::new(false));
            let id = shared.next_subscriber.fetch_add(1, Ordering::SeqCst);
            queue = Some((id, q_rx, Arc::clone(&overflow)));
            SubscriberQueue {
                id,
                tx: q_tx,
                overflow,
            }
        });
        if shared
            .requests
            .try_send(RemoteIncoming {
                request,
                reply: reply_tx,
                subscriber: queue_for_frame,
            })
            .is_err()
        {
            if answer(
                reader.get_mut(),
                slot.is_paired(),
                503,
                &error_body("busy", "the host is busy; try again"),
                &[],
            )? {
                continue;
            }
            return Ok(());
        }
        (shared.wake)();
        let reply = match reply_rx.recv_timeout(ANSWER_TIMEOUT) {
            Ok(reply) => reply,
            Err(_) => RemoteReply::new(
                504,
                error_body("no_answer", "the app did not answer in time"),
                false,
            ),
        };
        if reply.authenticated && !slot.is_paired() && !promote(&mut reader, slot, shared)? {
            return Ok(());
        }
        if reply.status == SUBSCRIBED
            && let (Some(key), Some((id, q_rx, overflow))) = (upgrade_key, queue)
        {
            respond_upgrade(reader.get_mut(), &key)?;
            let StreamOwned { conn, sock } = reader.into_inner();
            let stream = StreamOwned::new(conn, sock.into_inner());
            let out = subscriber(stream, &q_rx, &overflow, id, shared);
            let ended = SubscriberFrame {
                subscriber: id,
                text: None,
            };
            if shared.frames.send(ended).is_ok() {
                (shared.wake)();
            }
            return out;
        }
        if !answer(
            reader.get_mut(),
            slot.is_paired(),
            reply.status,
            &reply.body,
            &reply.headers,
        )? {
            return Ok(());
        }
    }
    Ok(())
}

/// Move a connection whose request authenticated from the
/// unauthenticated pool to the paired one, and from the deadlines to
/// the idle timeout. `false` when the paired pool is full: the device
/// has been told so and the connection is to close.
fn promote(reader: &mut BufReader<Tls>, slot: &mut Slot, shared: &Shared) -> io::Result<bool> {
    if shared.pools.paired.load(Ordering::SeqCst) >= MAX_PAIRED {
        answer(
            reader.get_mut(),
            false,
            503,
            &error_body("full", "too many paired connections"),
            &[],
        )?;
        return Ok(false);
    }
    slot.promote();
    let sock = &mut reader.get_mut().sock;
    sock.deadline = None;
    sock.sock.set_read_timeout(Some(IDLE_TIMEOUT))?;
    Ok(true)
}

/// Feed one WebSocket from its queue until the frame drops the queue
/// (a revocation, or an overflow, which ends with `resync_required`)
/// or the device closes. The socket is non-blocking: an event goes out
/// the moment it is queued, the device's own frames are looked for
/// only when there is nothing to send (a close ends the subscription,
/// a ping is answered by the library, a text frame goes to the frame
/// thread as a `SubscriberFrame`), and a device that stops reading
/// stalls only this thread, for `WRITE_TIMEOUT` at most, while the
/// frame fills and then drops its queue.
fn subscriber(
    stream: StreamOwned<ServerConnection, TcpStream>,
    events: &Receiver<String>,
    overflow: &AtomicBool,
    id: u64,
    shared: &Shared,
) -> io::Result<()> {
    stream.sock.set_nonblocking(true)?;
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(SUBSCRIBER_FRAME_MAX))
        .max_frame_size(Some(SUBSCRIBER_FRAME_MAX));
    let mut ws = tungstenite::WebSocket::from_raw_socket(
        stream,
        tungstenite::protocol::Role::Server,
        Some(config),
    );
    loop {
        match events.recv_timeout(SUBSCRIBER_POLL) {
            Ok(text) => {
                send_text(&mut ws, text)?;
                continue;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if overflow.load(Ordering::SeqCst) {
                    let _ = send_text(&mut ws, "{\"type\":\"resync_required\"}".into());
                }
                let _ = ws.close(None);
                let _ = flush_by(&mut ws, Instant::now() + WRITE_TIMEOUT);
                return Ok(());
            }
        }
        match ws.read() {
            Ok(tungstenite::Message::Close(_)) => return Ok(()),
            Ok(tungstenite::Message::Text(text)) => {
                let frame = SubscriberFrame {
                    subscriber: id,
                    text: Some(text.to_string()),
                };
                if shared.frames.try_send(frame).is_ok() {
                    (shared.wake)();
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(e) => return Err(io::Error::other(e.to_string())),
        }
    }
}

type Socket = tungstenite::WebSocket<StreamOwned<ServerConnection, TcpStream>>;

/// Queue one text frame and write it out, waiting up to `WRITE_TIMEOUT`
/// for a full socket to drain.
fn send_text(ws: &mut Socket, text: String) -> io::Result<()> {
    match ws.write(tungstenite::Message::Text(text.into())) {
        Ok(()) => {}
        Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {}
        Err(e) => return Err(io::Error::other(e.to_string())),
    }
    flush_by(ws, Instant::now() + WRITE_TIMEOUT)
}

/// Flush what is queued, retrying a full socket until `deadline`.
fn flush_by(ws: &mut Socket, deadline: Instant) -> io::Result<()> {
    loop {
        match ws.flush() {
            Ok(()) => return Ok(()),
            Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the device stopped reading",
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(io::Error::other(e.to_string())),
        }
    }
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

    /// A source that serves `prefix` and then `fill` bytes of `a`, and
    /// counts what was taken from it.
    struct Counting {
        prefix: Vec<u8>,
        fill: usize,
        taken: usize,
    }

    impl Read for Counting {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let total = self.prefix.len() + self.fill;
            let n = buf.len().min(total - self.taken);
            for (i, b) in buf[..n].iter_mut().enumerate() {
                *b = self.prefix.get(self.taken + i).copied().unwrap_or(b'a');
            }
            self.taken += n;
            Ok(n)
        }
    }

    /// The most a refused head may pull from the stream: the head budget,
    /// the byte that proves it over, and one `BufReader` fill.
    const HEAD_READ_MAX: usize = MAX_HEAD + 1 + 8 * 1024;

    fn refused_head(prefix: &[u8]) {
        let mut r = BufReader::new(Counting {
            prefix: prefix.to_vec(),
            fill: 1024 * 1024,
            taken: 0,
        });
        let err = read_request(&mut r).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        let taken = r.get_ref().taken;
        assert!(taken <= HEAD_READ_MAX, "read {taken} bytes of the head");
    }

    #[test]
    fn a_request_line_with_no_newline_stops_at_the_head_bound() {
        refused_head(b"");
    }

    #[test]
    fn a_header_with_no_newline_stops_at_the_head_bound() {
        refused_head(b"GET /x HTTP/1.1\r\nX-Pad: ");
    }

    #[test]
    fn a_head_of_exactly_the_bound_parses_and_one_byte_more_does_not() {
        let head = |pad: usize| {
            let mut text = b"GET /x HTTP/1.1\r\nX-Pad: ".to_vec();
            text.extend(std::iter::repeat_n(b'a', pad));
            text.extend_from_slice(b"\r\n\r\n");
            text
        };
        let pad = MAX_HEAD - head(0).len();
        let exact = head(pad);
        assert_eq!(exact.len(), MAX_HEAD);
        let mut r = BufReader::new(&exact[..]);
        assert!(read_request(&mut r).unwrap().is_some());
        let over = head(pad + 1);
        let mut r = BufReader::new(&over[..]);
        assert!(read_request(&mut r).is_err());
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

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn one_source_holds_its_share_and_another_is_admitted_up_to_the_total() {
        let mut a = Admission::new(MAX_UNAUTHENTICATED, MAX_UNAUTHENTICATED_PER_SOURCE);
        let stranger = ip("192.168.1.50");
        for _ in 0..MAX_UNAUTHENTICATED_PER_SOURCE {
            assert_eq!(a.admit(stranger), Ok(()));
        }
        assert_eq!(a.admit(stranger), Err(Refusal::Source));
        // A fresh device on another address still finds a slot.
        assert_eq!(a.admit(ip("192.168.1.20")), Ok(()));
        assert_eq!(a.admit(ip("192.168.1.21")), Err(Refusal::Total));
        a.release(stranger);
        assert_eq!(a.admit(ip("192.168.1.21")), Ok(()));
        assert_eq!(a.admit(stranger), Err(Refusal::Total));
        a.release(ip("192.168.1.20"));
        a.release(ip("192.168.1.21"));
        assert_eq!(a.admit(stranger), Ok(()), "its own share is back");
        assert_eq!(a.admit(stranger), Err(Refusal::Source));
    }

    #[test]
    fn a_mapped_ipv4_address_and_its_plain_form_are_one_source() {
        let mut a = Admission::new(MAX_UNAUTHENTICATED, 2);
        assert_eq!(a.admit(ip("::ffff:10.0.0.7")), Ok(()));
        assert_eq!(a.admit(ip("10.0.0.7")), Ok(()));
        assert_eq!(a.admit(ip("::ffff:10.0.0.7")), Err(Refusal::Source));
        a.release(ip("10.0.0.7"));
        a.release(ip("::ffff:10.0.0.7"));
        assert_eq!((a.total, a.by_source.len()), (0, 0));
        // A release of what was never admitted changes nothing.
        a.release(ip("10.0.0.8"));
        assert_eq!(a.total, 0);
    }

    fn unauthenticated(pools: &Pools) -> usize {
        pools.unauthenticated.lock().unwrap().total
    }

    #[test]
    fn a_slot_gives_back_whichever_pool_it_is_in() {
        let pools = Arc::new(Pools::new());
        let source = ip("10.0.0.7");
        let slot = Slot::take(&pools, source).unwrap();
        assert_eq!(unauthenticated(&pools), 1);
        drop(slot);
        assert_eq!(unauthenticated(&pools), 0);

        let mut slot = Slot::take(&pools, source).unwrap();
        slot.promote();
        assert!(slot.is_paired());
        assert_eq!(unauthenticated(&pools), 0);
        assert_eq!(pools.paired.load(Ordering::SeqCst), 1);
        drop(slot);
        assert_eq!(pools.paired.load(Ordering::SeqCst), 0);
        assert_eq!(unauthenticated(&pools), 0);

        let slot = Slot::take(&pools, source).unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _held = slot;
            panic!("a connection thread panics");
        }));
        assert!(panicked.is_err());
        assert_eq!(unauthenticated(&pools), 0, "the unwind gave the slot back");
    }

    /// A listener on loopback with 300 ms deadlines, and a client
    /// pinned to it.
    fn short_port() -> (RemotePort, Arc<ClientConfig>) {
        let (identity, key) = make_identity(None, "Test Mac", Listen::default()).unwrap();
        let tls = tls_config(&identity, &key).unwrap();
        let ms = Duration::from_millis(300);
        let timeouts = Timeouts {
            handshake: ms,
            head: ms,
            body: ms,
        };
        let port =
            RemotePort::bind_with("127.0.0.1:0".parse().unwrap(), tls, || {}, timeouts).unwrap();
        (port, pinned_client(&identity.spki_sha256).unwrap())
    }

    type Client = StreamOwned<rustls::ClientConnection, TcpStream>;

    fn client(config: &Arc<ClientConfig>, tcp: TcpStream) -> Client {
        let conn = rustls::ClientConnection::new(
            Arc::clone(config),
            ServerName::try_from("localhost").unwrap(),
        )
        .unwrap();
        StreamOwned::new(conn, tcp)
    }

    fn handshaken(addr: SocketAddr, config: &Arc<ClientConfig>) -> Client {
        let mut s = client(config, TcpStream::connect(addr).unwrap());
        while s.conn.is_handshaking() {
            s.conn.complete_io(&mut s.sock).unwrap();
        }
        s
    }

    /// The TLS bytes that carry `plain` over `s`, to be written by hand.
    fn sealed(s: &mut Client, plain: &[u8]) -> Vec<u8> {
        s.conn.writer().write_all(plain).unwrap();
        let mut out = Vec::new();
        while s.conn.wants_write() {
            s.conn.write_tls(&mut out).unwrap();
        }
        out
    }

    /// Write `bytes` to `tcp` one every 100 ms, on its own thread, and
    /// wait for the port to close: how long it took from `from`, or
    /// `None` if the port was still reading after three seconds.
    fn trickled(tcp: &TcpStream, bytes: Vec<u8>, from: Instant) -> Option<Duration> {
        let mut writer = tcp.try_clone().unwrap();
        std::thread::spawn(move || {
            for b in bytes {
                if writer.write_all(&[b]).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let mut reader = tcp.try_clone().unwrap();
        reader
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return Some(from.elapsed()),
                // A TLS 1.3 server sends its tickets after the
                // handshake; anything else read is not a close yet.
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return None;
                }
                Err(_) => return Some(from.elapsed()),
            }
        }
    }

    #[test]
    fn a_trickled_handshake_head_or_body_is_closed_at_its_deadline_and_frees_its_slot() {
        let (port, config) = short_port();
        let addr = port.addr();
        let handshake = std::thread::spawn({
            let config = Arc::clone(&config);
            move || {
                let tcp = TcpStream::connect(addr).unwrap();
                let mut hello = Vec::new();
                client(&config, tcp.try_clone().unwrap())
                    .conn
                    .write_tls(&mut hello)
                    .unwrap();
                trickled(&tcp, hello, Instant::now())
            }
        });
        let head = std::thread::spawn({
            let config = Arc::clone(&config);
            move || {
                let mut s = handshaken(addr, &config);
                let from = Instant::now();
                let bytes = sealed(&mut s, b"GET /app.css HTTP/1.1\r\nHost: localhost\r\n\r\n");
                trickled(&s.sock, bytes, from)
            }
        });
        let body = std::thread::spawn({
            let config = Arc::clone(&config);
            move || {
                let mut s = handshaken(addr, &config);
                write!(
                    s,
                    "POST /api/v1/pair HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1024\r\n\r\n"
                )
                .unwrap();
                s.flush().unwrap();
                let from = Instant::now();
                let bytes = sealed(&mut s, &[b'a'; 1024]);
                trickled(&s.sock, bytes, from)
            }
        });
        // The three hold their address's share, so a fourth is closed
        // at once.
        std::thread::sleep(Duration::from_millis(100));
        let mut fourth = TcpStream::connect(addr).unwrap();
        fourth
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut buf = [0u8; 1];
        assert!(
            matches!(fourth.read(&mut buf), Ok(0) | Err(_)),
            "a fourth from the address was closed at accept"
        );
        drop(fourth);
        for (phase, held) in [("handshake", handshake), ("head", head), ("body", body)] {
            let closed = held.join().unwrap();
            let elapsed = closed.unwrap_or_else(|| panic!("the {phase} was still being read"));
            assert!(
                elapsed < Duration::from_secs(2),
                "the {phase} was closed after {elapsed:?}"
            );
        }
        // Each gave its slot back: a page load from the address is
        // served.
        std::thread::sleep(Duration::from_millis(50));
        let mut s = handshaken(addr, &config);
        write!(s, "GET /app.css HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        s.flush().unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).unwrap();
        assert!(String::from_utf8_lossy(&all).starts_with("HTTP/1.1 200"));
    }
}
