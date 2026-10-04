//! v1: [`client_config`], [`server_config`], [`pinned_client_config`], [`cpu_check`], [`install_default`].

use std::sync::{Arc, OnceLock};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme};
use rustls_platform_verifier::Verifier;

#[derive(Debug, Clone, thiserror::Error)]
pub enum TlsError {
    /// Graviola needs these CPU features (most x86-64 CPUs since about 2014, ARMv8 with the crypto extensions).
    #[error("this CPU lacks features TLS needs: {}", .0.join(", "))]
    Cpu(Vec<&'static str>),
    #[error("TLS setup failed: {0}")]
    Setup(String),
}

/// The CPU features graviola needs that this CPU does not have (empty = fine).
pub fn missing_cpu_features() -> Vec<&'static str> {
    #[cfg(target_arch = "x86_64")]
    {
        let mut missing = Vec::new();
        macro_rules! need {
            ($($f:tt),*) => {$(
                if !std::arch::is_x86_feature_detected!($f) {
                    missing.push($f);
                }
            )*};
        }
        need!("aes", "ssse3", "avx", "avx2", "adx", "bmi2", "pclmulqdq");
        missing
    }
    #[cfg(target_arch = "aarch64")]
    {
        let mut missing = Vec::new();
        macro_rules! need {
            ($($f:tt),*) => {$(
                if !std::arch::is_aarch64_feature_detected!($f) {
                    missing.push($f);
                }
            )*};
        }
        need!("aes", "sha2", "pmull", "neon");
        missing
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        vec!["an x86-64 or aarch64 CPU"]
    }
}

/// Fails when TLS cannot work on this CPU (checked once at start; the bot exits with a clear message).
pub fn cpu_check() -> Result<(), TlsError> {
    let missing = missing_cpu_features();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(TlsError::Cpu(missing))
    }
}

fn provider() -> Arc<CryptoProvider> {
    static P: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    P.get_or_init(|| Arc::new(rustls_graviola::default_provider())).clone()
}

/// Makes graviola the process-wide default provider (for libraries that build their own rustls configs without
/// naming a provider). Harmless when called twice.
pub fn install_default() -> Result<(), TlsError> {
    cpu_check()?;
    let _ = rustls_graviola::default_provider().install_default();
    Ok(())
}

/// The client configuration every connection uses (built once).
pub fn client_config() -> Result<Arc<ClientConfig>, TlsError> {
    static C: OnceLock<Result<Arc<ClientConfig>, TlsError>> = OnceLock::new();
    C.get_or_init(|| {
        cpu_check()?;
        let provider = provider();
        let verifier = Verifier::new_with_extra_roots(
            webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned(),
            provider.clone(),
        )
        .map_err(|e| TlsError::Setup(e.to_string()))?;
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsError::Setup(e.to_string()))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        Ok(Arc::new(config))
    })
    .clone()
}

/// The certificates of a PEM file (the server's first, then its chain).
fn chain(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, TlsError> {
    let chain = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::Setup(format!("the certificate file: {e}")))?;
    if chain.is_empty() {
        return Err(TlsError::Setup("the certificate file holds no certificate".into()));
    }
    Ok(chain)
}

/// The web UI's HTTPS configuration from PEM files: the certificate (then its chain) and its private key.
pub fn server_config(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<ServerConfig>, TlsError> {
    cpu_check()?;
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| TlsError::Setup(format!("the key file: {e}")))?;
    let mut config = ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Setup(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(chain(cert_pem)?, key)
        .map_err(|e| TlsError::Setup(e.to_string()))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// A client that trusts exactly the server certificate in `cert_pem`, whatever names it carries (the bot asking
/// its own web UI on this machine).
pub fn pinned_client_config(cert_pem: &[u8]) -> Result<Arc<ClientConfig>, TlsError> {
    cpu_check()?;
    let provider = provider();
    let pinned = Pinned {
        cert: chain(cert_pem)?.swap_remove(0),
        algorithms: provider.signature_verification_algorithms,
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Setup(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(pinned))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[derive(Debug)]
struct Pinned {
    cert: CertificateDer<'static>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if end_entity.as_ref() == self.cert.as_ref() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: &[u8] = include_bytes!("../../../pb-testkit/fixtures/tls/cert.pem");
    const KEY: &[u8] = include_bytes!("../../../pb-testkit/fixtures/tls/key.pem");

    #[test]
    fn builds_on_this_machine() {
        assert_eq!(missing_cpu_features(), Vec::<&str>::new());
        let c = client_config().map_err(|e| e.to_string());
        assert!(c.is_ok(), "{c:?}");
    }

    #[test]
    fn serves_and_pins_a_certificate() {
        assert!(server_config(CERT, KEY).is_ok());
        assert!(pinned_client_config(CERT).is_ok());
        assert!(server_config(KEY, KEY).is_err(), "a key is no certificate");
        assert!(server_config(CERT, CERT).is_err(), "a certificate is no key");
    }
}
