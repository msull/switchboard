//! What the server and the bench client share: the certificate pin, a
//! minimal HTTP/1.1 request reader, and the WebSocket accept line.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::Arc;

use base64::Engine as _;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha1::Sha1;
use sha2::{Digest, Sha256};

/// The crypto provider every connection uses; `ring`, which the app's
/// dependency tree already builds.
#[must_use]
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// SHA-256 over the certificate's DER. The design pins the SPKI hash so
/// a reissued certificate over the same key still verifies; the spike
/// pins the whole certificate, which exercises the same mechanism on
/// the client (a verifier that checks one hash and nothing else).
#[must_use]
pub fn fingerprint(cert: &CertificateDer<'_>) -> [u8; 32] {
    Sha256::digest(cert.as_ref()).into()
}

/// A client verifier that accepts exactly one certificate, by hash.
/// Nothing about names, dates or chains is consulted: the pin is the
/// trust.
#[derive(Debug)]
pub struct Pin {
    pub fingerprint: [u8; 32],
    pub provider: Arc<CryptoProvider>,
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
        if fingerprint(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "certificate is not the pinned one".into(),
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

/// One HTTP/1.1 request, as much of it as the port needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// Header names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// The most a request line and its headers may take, and the most a
/// body may be (the design says 64 KiB).
pub const MAX_HEAD: usize = 8 * 1024;
pub const MAX_BODY: usize = 64 * 1024;

/// Read one request. Returns `None` at a clean end of stream before
/// any byte; an oversized or malformed request is an error.
pub fn read_request<R: Read>(reader: &mut BufReader<R>) -> io::Result<Option<Request>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad request line",
        ));
    };
    let mut headers = Vec::new();
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
    let length: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(Request {
        method: method.to_owned(),
        path: path.to_owned(),
        headers,
        body,
    }))
}

/// Write a response with a body.
pub fn respond<W: Write>(w: &mut W, status: u16, reason: &str, body: &[u8]) -> io::Result<()> {
    // Head and body in one write, so the reply is one TLS record and
    // one segment.
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    w.write_all(&out)?;
    w.flush()
}

/// The `Sec-WebSocket-Accept` value for a client key (RFC 6455 §4.2.2).
#[must_use]
pub fn websocket_accept(key: &str) -> String {
    let mut h = Sha1::new();
    h.update(key.as_bytes());
    h.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    base64::engine::general_purpose::STANDARD.encode(h.finalize())
}

/// Write the 101 that switches a connection to WebSocket. The request
/// was already read by `read_request`, so `tungstenite::accept` cannot
/// be used; the stream is wrapped with `from_raw_socket` after this.
pub fn respond_upgrade<W: Write>(w: &mut W, key: &str) -> io::Result<()> {
    write!(
        w,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        websocket_accept(key)
    )?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rfc_example_key_accepts() {
        assert_eq!(
            websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn a_request_reads_with_its_body() {
        let text = b"POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 2\r\n\r\nhi";
        let mut r = BufReader::new(&text[..]);
        let req = read_request(&mut r).unwrap().unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/x");
        assert_eq!(req.header("host"), Some("h"));
        assert_eq!(req.body, b"hi");
        assert!(read_request(&mut r).unwrap().is_none());
    }

    #[test]
    fn an_oversized_body_is_refused_before_it_is_read() {
        let text = b"POST /x HTTP/1.1\r\nContent-Length: 100000\r\n\r\n";
        let mut r = BufReader::new(&text[..]);
        assert!(read_request(&mut r).is_err());
    }
}
