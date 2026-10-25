use std::fmt;
use std::str::FromStr;

use sha2::{Digest, Sha256};

/// The SHA-256 of a server's certificate, which clients pin the first time they connect.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    /// The fingerprint of a DER-encoded certificate.
    pub fn of(certificate: &[u8]) -> Self {
        Self(Sha256::digest(certificate).into())
    }
}

/// Lowercase hex pairs joined by colons, as `ssh-keygen -l` and browsers show them.
impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(":")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("a fingerprint is 32 hex pairs joined by colons")]
pub struct ParseFingerprintError;

impl FromStr for Fingerprint {
    type Err = ParseFingerprintError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0; 32];
        let mut pairs = text.trim().split(':');
        for byte in &mut bytes {
            // from_str_radix alone would take a sign, as in "+f".
            let pair = pairs.next().filter(|pair| pair.len() == 2 && pair.bytes().all(|byte| byte.is_ascii_hexdigit()));
            let pair = pair.ok_or(ParseFingerprintError)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| ParseFingerprintError)?;
        }
        if pairs.next().is_some() {
            return Err(ParseFingerprintError);
        }
        Ok(Self(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_it_writes() {
        let fingerprint = Fingerprint::of(b"certificate");
        assert_eq!(fingerprint.to_string().parse::<Fingerprint>().unwrap(), fingerprint);
    }

    #[test]
    fn turns_away_a_short_or_signed_one() {
        assert!("ab:cd".parse::<Fingerprint>().is_err());
        let signed = Fingerprint::of(b"certificate").to_string().replacen(|digit: char| digit.is_ascii_hexdigit(), "+", 1);
        assert!(signed.parse::<Fingerprint>().is_err());
    }
}
