//! Proxmox's own certificate, pinned the way SSH identities are.
//!
//! A Proxmox host makes its own certificate when it is installed, so no
//! authority vouches for it and the usual checks would always fail. Turning
//! the checks off would let anything on the network pose as the Proxmox host
//! and collect the API token. Instead the owner is shown the certificate's
//! SHA-256 fingerprint on first contact, checks it against the one on the host
//! itself, and approves it. After that only that exact certificate is
//! accepted; any other one stops the connection before anything is sent.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, SignatureScheme};

/// SHA-256 of a certificate, as "AB:CD:…" in capitals: the same text that
/// `openssl x509 -noout -fingerprint -sha256` and `pvenode cert info` print.
pub fn fingerprint(der: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, der);
    digest.as_ref().iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// True if two fingerprints name the same certificate. Only the hex digits
/// count, in either case, so a fingerprint copied with or without colons or
/// "sha256 Fingerprint=" in front still matches.
pub fn same_fingerprint(a: &str, b: &str) -> bool {
    let digits = |s: &str| -> String {
        let s = s.trim();
        // Drop a label such as "sha256 Fingerprint=" or "SHA256:" first: its letters a-f would count.
        let s = s.rsplit(['=', ' ']).next().unwrap_or(s);
        let s = s.strip_prefix("SHA256:").or_else(|| s.strip_prefix("sha256:")).unwrap_or(s);
        s.chars().filter(|c| c.is_ascii_hexdigit()).map(|c| c.to_ascii_uppercase()).collect()
    };
    let (a, b) = (digits(a), digits(b));
    a.len() == 64 && a == b
}

/// The ring crypto provider. No other one is built in: it needs no extra
/// build tools on Windows.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts exactly one certificate, the pinned one, and notes what the
/// server presented so the owner can be asked about it.
#[derive(Debug)]
pub struct PinnedCert {
    pinned: Option<String>,
    seen: Mutex<Option<String>>,
    provider: Arc<CryptoProvider>,
}

impl PinnedCert {
    /// `pinned` is the fingerprint the owner approved, or None if they haven't
    /// approved one yet, in which case every certificate is turned away.
    pub fn new(pinned: Option<String>) -> Arc<Self> {
        Arc::new(Self { pinned, seen: Mutex::new(None), provider: provider() })
    }

    /// The fingerprint of the certificate the server presented, once the
    /// connection got that far.
    pub fn seen(&self) -> Option<String> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// True if the server presented a certificate and it wasn't the pinned one.
    pub fn refused(&self) -> bool {
        match (self.seen(), &self.pinned) {
            (Some(seen), Some(pinned)) => !same_fingerprint(&seen, pinned),
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// TLS settings that use this check and nothing else.
    pub fn client_config(self: &Arc<Self>) -> Result<ClientConfig, rustls::Error> {
        let mut config = ClientConfig::builder_with_provider(self.provider.clone())
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(self.clone())
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(config)
    }
}

impl ServerCertVerifier for PinnedCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // The name and dates aren't checked: the owner approved this very
        // certificate, which is a stronger check than either. The handshake
        // signature still is (below), so the server must hold its private key.
        let seen = fingerprint(end_entity.as_ref());
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(seen.clone());
        match &self.pinned {
            Some(pinned) if same_fingerprint(pinned, &seen) => Ok(ServerCertVerified::assertion()),
            _ => Err(rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::pem::PemObject;

    /// What `openssl x509 -noout -fingerprint -sha256 -in testdata/cert_a.pem` prints.
    const CERT_A_PRINT: &str =
        "EC:8E:F3:EE:05:91:6C:C0:58:72:9F:5C:73:34:77:8C:70:80:06:EE:49:B9:77:DA:EC:1C:7A:D0:97:1C:9A:93";

    fn cert_a() -> CertificateDer<'static> {
        CertificateDer::from_pem_slice(include_bytes!("testdata/cert_a.pem")).unwrap()
    }

    #[test]
    fn fingerprint_matches_openssl() {
        assert_eq!(fingerprint(cert_a().as_ref()), CERT_A_PRINT);
    }

    #[test]
    fn fingerprints_compare_by_their_digits() {
        let lower = CERT_A_PRINT.to_lowercase();
        let bare: String = CERT_A_PRINT.replace(':', "");
        assert!(same_fingerprint(CERT_A_PRINT, &lower));
        assert!(same_fingerprint(CERT_A_PRINT, &bare));
        assert!(same_fingerprint(CERT_A_PRINT, &format!("sha256 Fingerprint={CERT_A_PRINT}")));
        assert!(same_fingerprint(&format!("  {CERT_A_PRINT}\n"), CERT_A_PRINT));
        // One digit different, cut short, or empty: not the same.
        let mut other = CERT_A_PRINT.to_string();
        other.replace_range(0..1, "F");
        assert!(!same_fingerprint(CERT_A_PRINT, &other));
        assert!(!same_fingerprint(CERT_A_PRINT, &CERT_A_PRINT[..40]));
        assert!(!same_fingerprint("", ""));
    }

    fn check(verifier: &PinnedCert) -> Result<ServerCertVerified, rustls::Error> {
        verifier.verify_server_cert(&cert_a(), &[], &ServerName::try_from("192.168.68.30").unwrap(), &[], UnixTime::now())
    }

    #[test]
    fn only_the_pinned_certificate_is_accepted() {
        // Nothing approved yet: refused, and what it showed is kept for the owner to judge.
        let none = PinnedCert::new(None);
        assert!(!none.refused(), "nothing seen yet");
        assert!(check(&none).is_err());
        assert_eq!(none.seen().as_deref(), Some(CERT_A_PRINT));
        assert!(none.refused());

        let right = PinnedCert::new(Some(CERT_A_PRINT.to_lowercase()));
        assert!(check(&right).is_ok());
        assert!(!right.refused());

        let wrong = PinnedCert::new(Some("00:".repeat(31) + "00"));
        assert!(check(&wrong).is_err());
        assert_eq!(wrong.seen().as_deref(), Some(CERT_A_PRINT));
        assert!(wrong.refused());
    }
}
