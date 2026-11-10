//! One HTTPS exchange with a host's remote port over the pinned
//! client: a fresh connection per request, the request written whole,
//! the reply read by its `Content-Length`, then the connection dropped.
//! std and rustls only; a general HTTP client cannot take the pinned
//! verifier.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs as _};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use crate::pin::{PIN_MISMATCH, pinned_client};

/// The most a reply body may be; the port caps its own at 1 MiB.
const REPLY_CAP: usize = 2 * 1024 * 1024;

/// How long one exchange may take, connect to last byte.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// A TLS stream to a host, handshaken.
pub type Tls = StreamOwned<ClientConnection, TcpStream>;

/// Why an exchange did not get a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The host showed a key other than the pinned one.
    Pin,
    /// Anything else: no route, a refused connection, a cut reply.
    Io(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pin => f.write_str(PIN_MISMATCH),
            Self::Io(e) => f.write_str(e),
        }
    }
}

/// A reply: its status and its JSON (null for an empty or non-JSON
/// body).
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub json: serde_json::Value,
}

impl Reply {
    /// The envelope's `body`.
    #[must_use]
    pub fn body(&self) -> &serde_json::Value {
        &self.json["body"]
    }

    /// The refusal's code and message, for a status that is not 2xx.
    #[must_use]
    pub fn error(&self) -> (String, String) {
        let e = &self.json["error"];
        (
            e["code"].as_str().unwrap_or("failed").to_owned(),
            e["message"].as_str().unwrap_or("").to_owned(),
        )
    }

    #[must_use]
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

fn io(e: impl std::fmt::Display) -> Failure {
    let text = e.to_string();
    if text.contains(PIN_MISMATCH) {
        Failure::Pin
    } else {
        Failure::Io(text)
    }
}

/// A TLS connection to the first of `addresses` that answers, pinned
/// to `spki_sha256`, with `timeout` on every read and write.
pub fn connect(addresses: &[String], spki_sha256: &str, timeout: Duration) -> Result<Tls, Failure> {
    let config: Arc<ClientConfig> = pinned_client(spki_sha256).map_err(Failure::Io)?;
    let mut last = Failure::Io("no address".into());
    for address in addresses {
        let addrs: Vec<SocketAddr> = match address.to_socket_addrs() {
            Ok(a) => a.collect(),
            Err(e) => {
                last = Failure::Io(format!("{address}: {e}"));
                continue;
            }
        };
        for addr in addrs {
            match handshake(addr, &config, timeout) {
                Ok(tls) => return Ok(tls),
                Err(Failure::Pin) => return Err(Failure::Pin),
                Err(e) => last = e,
            }
        }
    }
    Err(last)
}

fn handshake(
    addr: SocketAddr,
    config: &Arc<ClientConfig>,
    timeout: Duration,
) -> Result<Tls, Failure> {
    let tcp = TcpStream::connect_timeout(&addr, timeout).map_err(io)?;
    tcp.set_read_timeout(Some(timeout)).map_err(io)?;
    tcp.set_write_timeout(Some(timeout)).map_err(io)?;
    // The pin is the identity; the name is never checked.
    let name = ServerName::try_from("localhost").map_err(io)?;
    let conn = ClientConnection::new(Arc::clone(config), name).map_err(io)?;
    let mut s = StreamOwned::new(conn, tcp);
    while s.conn.is_handshaking() {
        s.conn.complete_io(&mut s.sock).map_err(io)?;
    }
    Ok(s)
}

/// One request; `body` is sent as JSON when given.
pub fn request(
    addresses: &[String],
    spki_sha256: &str,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<&serde_json::Value>,
) -> Result<Reply, Failure> {
    let mut s = connect(addresses, spki_sha256, TIMEOUT)?;
    let text = body.map(serde_json::Value::to_string).unwrap_or_default();
    let auth = bearer
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        text.len()
    );
    s.write_all(head.as_bytes()).map_err(io)?;
    s.write_all(text.as_bytes()).map_err(io)?;
    s.flush().map_err(io)?;
    let mut reader = BufReader::new(s);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(io)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| Failure::Io(format!("not an HTTP reply: {:?}", line.trim_end())))?;
    let mut length = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).map_err(io)? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((name, value)) = h.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut out = Vec::new();
    match length {
        Some(n) if n > REPLY_CAP => return Err(Failure::Io(format!("a reply of {n} bytes"))),
        Some(n) => {
            out.resize(n, 0);
            reader.read_exact(&mut out).map_err(io)?;
        }
        None => {
            reader
                .take(REPLY_CAP as u64)
                .read_to_end(&mut out)
                .map_err(io)?;
        }
    }
    let json = serde_json::from_slice(&out).unwrap_or(serde_json::Value::Null);
    Ok(Reply { status, json })
}
