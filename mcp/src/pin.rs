//! The pinned TLS client: it trusts exactly one public key, by the
//! SHA-256 of its `SubjectPublicKeyInfo`, and nothing else (no names,
//! no dates, no chain), as every paired device does. A copy of the
//! app's `adapters::remote::pinned_client`, so this helper does not
//! link the app and its windowing stack. The two must stay alike: a
//! fix to the verifier in one reaches the other in the same commit
//! (the Layout table in the README says so beside both files).

use std::fmt::Write as _;
use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// What a refused key says, so a caller can tell a rotated host key
/// from a network failure.
pub const PIN_MISMATCH: &str = "the host's key is not the pinned one";

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// A client configuration that trusts only the key hashing to
/// `spki_sha256` (lowercase hex).
pub fn pinned_client(spki_sha256: &str) -> Result<Arc<ClientConfig>, String> {
    let provider = provider();
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pin {
            spki_sha256: spki_sha256.trim().to_ascii_lowercase(),
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
            Err(rustls::Error::General(PIN_MISMATCH.into()))
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
/// hashed.
#[must_use]
pub fn spki_sha256_of_cert(der: &[u8]) -> Option<String> {
    let (0x30, tbs_and_rest, _) = der_element(der)? else {
        return None;
    };
    let (0x30, mut rest, _) = der_element(tbs_and_rest)? else {
        return None;
    };
    // The version is explicit and optional.
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

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::PublicKeyData as _;

    #[test]
    fn the_hash_is_of_the_certificates_public_key() {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        assert_eq!(
            spki_sha256_of_cert(cert.der()),
            Some(sha256_hex(&key.subject_public_key_info()))
        );
        assert_eq!(spki_sha256_of_cert(b"\x30\x01"), None);
    }
}
