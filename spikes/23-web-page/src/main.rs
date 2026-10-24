//! A throwaway HTTPS server with the shape the remote port's page
//! needs, to be opened in a browser by hand: a self-signed P-256
//! certificate with no subject alternative name (as `make_identity`
//! makes it), a page under the port's Content-Security-Policy, a
//! `/pair` that sets `__Host-sb-<id>` as an `HttpOnly` cookie, a `/me`
//! that says whether that cookie came back, and a WebSocket upgrade at
//! `/events` that sends one frame and closes, so the page can report
//! whether `wss:` to this certificate is admitted under `connect-src
//! 'self'`.
//!
//! Run: `cargo run --release -- [--port 43187] [--id a]`. Two instances
//! on two ports with two ids show whether the cookies stay apart.
//! The page prints the three outcomes in words; nothing is logged.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use base64::Engine as _;
use rcgen::PublicKeyData as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use sha1::Digest as _;

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

const PAGE: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>Spike 23</title>
<link rel="icon" href="data:,"><script src="/app.js" defer></script></head>
<body><h1>Spike 23</h1>
<p>Server id: <span id="id"></span></p>
<p>Cookie after pairing: <span id="cookie">not asked</span></p>
<p>Cookie on reload: <span id="reload">not asked</span></p>
<p>Socket: <span id="socket">not tried</span></p>
<button id="pair" type="button">Pair</button>
<button id="me" type="button">Read /me</button>
<button id="ws" type="button">Open the socket</button>
</body></html>
"#;

const SCRIPT: &str = r"'use strict';
const $ = (id) => document.getElementById(id);
async function me() {
  const r = await fetch('/me', { credentials: 'same-origin' });
  return r.status + ' ' + (await r.text());
}
(async () => {
  const info = await (await fetch('/info')).json();
  $('id').textContent = info.id;
  $('reload').textContent = await me();
})();
$('pair').addEventListener('click', async () => {
  const r = await fetch('/pair', { method: 'POST', credentials: 'same-origin' });
  $('cookie').textContent = r.status + ' then /me: ' + (await me());
});
$('me').addEventListener('click', async () => { $('cookie').textContent = await me(); });
$('ws').addEventListener('click', () => {
  $('socket').textContent = 'connecting';
  let socket;
  try { socket = new WebSocket('wss://' + location.host + '/events'); }
  catch (e) { $('socket').textContent = 'constructor threw: ' + e.message; return; }
  socket.onopen = () => { $('socket').textContent = 'open'; };
  socket.onmessage = (m) => { $('socket').textContent = 'frame: ' + m.data; };
  socket.onclose = (e) => { $('socket').textContent += '; closed ' + e.code; };
  socket.onerror = () => { $('socket').textContent += '; error'; };
});
";

fn main() -> io::Result<()> {
    let mut port = 43187u16;
    let mut id = String::from("a");
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--port" => port = it.next().unwrap().parse().unwrap(),
            "--id" => id = it.next().unwrap(),
            other => panic!("unknown argument {other}"),
        }
    }
    let key = rcgen::KeyPair::generate().map_err(io::Error::other)?;
    let mut params =
        rcgen::CertificateParams::new(Vec::<String>::new()).map_err(io::Error::other)?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, format!("Switchboard spike {id}"));
    let cert = params.self_signed(&key).map_err(io::Error::other)?;
    let spki = sha2::Sha256::digest(key.subject_public_key_info());
    let hex = spki.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    });
    eprintln!("spike 23: id {id} on https://127.0.0.1:{port}/ key {hex}");
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(io::Error::other)?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.der().to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
            )
            .map_err(io::Error::other)?;
    let config = Arc::new(config);
    let id = Arc::new(id);
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (config, id) = (Arc::clone(&config), Arc::clone(&id));
        std::thread::spawn(move || {
            if let Err(e) = connection(stream, &config, &id) {
                eprintln!("connection: {e}");
            }
        });
    }
    Ok(())
}

fn connection(tcp: TcpStream, config: &Arc<ServerConfig>, id: &str) -> io::Result<()> {
    let conn = ServerConnection::new(Arc::clone(config)).map_err(io::Error::other)?;
    let mut stream = StreamOwned::new(conn, tcp);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
            return Ok(());
        };
        let (method, path) = (
            method.to_owned(),
            target.split('?').next().unwrap().to_owned(),
        );
        let mut cookie = String::new();
        let mut ws_key = String::new();
        let mut length = 0usize;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h)?;
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((n, v)) = h.split_once(':') {
                match n.trim().to_ascii_lowercase().as_str() {
                    "cookie" => v.trim().clone_into(&mut cookie),
                    "sec-websocket-key" => v.trim().clone_into(&mut ws_key),
                    "content-length" => length = v.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body)?;
        let name = format!("__Host-sb-{id}");
        let present = cookie
            .split(';')
            .filter_map(|p| p.trim().split_once('='))
            .any(|(n, v)| n == name && !v.is_empty());
        let w = reader.get_mut();
        match (method.as_str(), path.as_str()) {
            ("GET", "/") => page(w, "text/html; charset=utf-8", PAGE),
            ("GET", "/app.js") => page(w, "text/javascript; charset=utf-8", SCRIPT),
            ("GET", "/info") => json(w, 200, &format!("{{\"id\":\"{id}\"}}"), ""),
            ("POST", "/pair") => json(
                w,
                200,
                "{\"paired\":true}",
                &format!(
                    "Set-Cookie: {name}=token-for-{id}; Max-Age=34560000; Path=/; Secure; HttpOnly; SameSite=Strict\r\n"
                ),
            ),
            ("GET", "/me") if present => json(w, 200, &format!("{{\"cookie\":\"{name}\"}}"), ""),
            ("GET", "/me") => json(w, 401, "{\"error\":\"no cookie\"}", ""),
            ("GET", "/events") if !ws_key.is_empty() => {
                let accept = sha1::Sha1::digest(
                    format!("{ws_key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
                );
                let accept = base64::engine::general_purpose::STANDARD.encode(accept);
                write!(
                    w,
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                )?;
                // One text frame, "hello", then a close frame.
                w.write_all(&[0x81, 5, b'h', b'e', b'l', b'l', b'o', 0x88, 0])?;
                w.flush()?;
                return Ok(());
            }
            _ => json(w, 404, "{\"error\":\"not found\"}", ""),
        }?;
    }
}

fn page<W: Write>(w: &mut W, content_type: &str, body: &str) -> io::Result<()> {
    write!(
        w,
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nContent-Security-Policy: {CSP}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: keep-alive\r\n\r\n{body}",
        body.len()
    )?;
    w.flush()
}

fn json<W: Write>(w: &mut W, status: u16, body: &str, extra: &str) -> io::Result<()> {
    write!(
        w,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{extra}Connection: keep-alive\r\n\r\n{body}",
        body.len()
    )?;
    w.flush()
}
