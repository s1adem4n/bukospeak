//! Trusting the server by its certificate's fingerprint, as SSH trusts hosts: the first
//! connection pins it, and later ones only proceed while it stays the same.

use std::sync::{Arc, Mutex};

use bukospeak_proto::Fingerprint;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

#[derive(Debug)]
pub struct PinVerifier {
    pinned: Option<Fingerprint>,
    presented: Arc<Mutex<Option<Fingerprint>>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl PinVerifier {
    /// A verifier that accepts the certificate with the `pinned` fingerprint, or any if none is
    /// pinned yet. The returned slot receives the fingerprint the server presented.
    pub fn new(pinned: Option<Fingerprint>, provider: &CryptoProvider) -> (Self, Arc<Mutex<Option<Fingerprint>>>) {
        let presented = Arc::default();
        let verifier = Self { pinned, presented: Arc::clone(&presented), algorithms: provider.signature_verification_algorithms };
        (verifier, presented)
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let presented = Fingerprint::of(end_entity);
        *self.presented.lock().expect("the fingerprint lock is never poisoned") = Some(presented);
        match self.pinned {
            Some(pinned) if pinned != presented => {
                Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
