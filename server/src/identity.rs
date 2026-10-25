use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use anyhow::Context;
use bukospeak_proto::{Fingerprint, SERVER_NAME};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// The server's self-signed certificate and its key. Clients pin the certificate's fingerprint,
/// so it stays the same across restarts.
pub struct Identity {
    pub certificate: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
}

impl Identity {
    pub fn generate() -> anyhow::Result<Self> {
        let generated = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_owned()])?;
        Ok(Self { certificate: generated.cert.der().clone(), key: PrivatePkcs8KeyDer::from(generated.signing_key.serialize_der()).into() })
    }

    /// The identity in `dir`, created there on the first start.
    pub fn load_or_create(dir: &Path) -> anyhow::Result<Self> {
        let (certificate, key) = (dir.join("cert.der"), dir.join("key.der"));
        match (fs::read(&certificate), fs::read(&key)) {
            (Ok(certificate), Ok(key)) => Ok(Self { certificate: certificate.into(), key: PrivatePkcs8KeyDer::from(key).into() }),
            (Err(error), _) | (_, Err(error)) if error.kind() == ErrorKind::NotFound => {
                let identity = Self::generate()?;
                fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
                write_private(&key, identity.key.secret_der())?;
                fs::write(&certificate, &identity.certificate).with_context(|| format!("writing {}", certificate.display()))?;
                Ok(identity)
            }
            (Err(error), _) | (_, Err(error)) => Err(error).with_context(|| format!("reading the identity in {}", dir.display())),
        }
    }

    pub fn fingerprint(&self) -> Fingerprint {
        Fingerprint::of(&self.certificate)
    }
}

fn write_private(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(contents))
        .with_context(|| format!("writing {}", path.display()))
}
