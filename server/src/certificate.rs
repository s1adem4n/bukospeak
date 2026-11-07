//! The server's self-signed X.509 certificate for an Ed25519 key, written in DER as RFC 5280
//! lays it out, and made anew on each start: QUIC needs one, but the tailnet vouches for the
//! server, so clients take any. It carries only what TLS needs to parse it and check the
//! handshake's signature: the server's name, a validity without end, and the key.

use bukospeak_proto::SERVER_NAME;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OCTET_STRING: u8 = 0x04;
const OBJECT_IDENTIFIER: u8 = 0x06;
const UTF8_STRING: u8 = 0x0c;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;
/// The certificate's version, `[0]`, and its extensions, `[3]`, both explicitly tagged.
const VERSION: u8 = 0xa0;
const EXTENSIONS: u8 = 0xa3;
/// A subject alternative name's DNS name, `[2]`, implicitly tagged.
const DNS_NAME: u8 = 0x82;

/// Ed25519, as RFC 8410 identifies it: 1.3.101.112.
const ED25519: &[u8] = &[0x2b, 0x65, 0x70];
/// The common name of a distinguished name: 2.5.4.3.
const COMMON_NAME: &[u8] = &[0x55, 0x04, 0x03];
/// The subject alternative name extension: 2.5.29.17.
const SUBJECT_ALT_NAME: &[u8] = &[0x55, 0x1d, 0x11];
/// Version 3, which extensions need.
const V3: u8 = 2;
/// Valid from 1975, and for ever after: RFC 5280's time for a certificate without a
/// well-defined expiration.
const NOT_BEFORE: &[u8] = b"750101000000Z";
const NOT_AFTER: &[u8] = b"99991231235959Z";

/// The DER of a value of `tag` holding `contents`.
fn der(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match u8::try_from(contents.len()) {
        Ok(len) if len < 0x80 => out.push(len),
        _ => {
            let len = contents.len().to_be_bytes();
            let significant = &len[len.iter().position(|&byte| byte != 0).unwrap_or(len.len() - 1)..];
            out.push(0x80 | significant.len() as u8);
            out.extend_from_slice(significant);
        }
    }
    out.extend_from_slice(contents);
    out
}

fn sequence(parts: &[&[u8]]) -> Vec<u8> {
    der(SEQUENCE, &parts.concat())
}

/// A bit string of whole bytes.
fn bit_string(bytes: &[u8]) -> Vec<u8> {
    der(BIT_STRING, &[&[0][..], bytes].concat())
}

/// The distinguished name whose common name is `name`.
fn distinguished_name(name: &str) -> Vec<u8> {
    let common_name = sequence(&[&der(OBJECT_IDENTIFIER, COMMON_NAME), &der(UTF8_STRING, name.as_bytes())]);
    sequence(&[&der(SET, &common_name)])
}

/// A new key, and its certificate for [`SERVER_NAME`].
pub fn generate() -> anyhow::Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let random = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).map_err(|_| anyhow::anyhow!("the system has no randomness for a key"))?;
    let key =
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|rejected| anyhow::anyhow!("the key just generated is rejected: {rejected}"))?;
    let mut serial = [0; 16];
    random.fill(&mut serial).map_err(|_| anyhow::anyhow!("the system has no randomness for a serial number"))?;
    let certificate = self_signed(&key, SERVER_NAME, serial);
    Ok((certificate.into(), PrivatePkcs8KeyDer::from(pkcs8.as_ref().to_vec()).into()))
}

/// The certificate of `key`, signed by it, for the server `name`, with the `serial` number.
fn self_signed(key: &Ed25519KeyPair, name: &str, serial: [u8; 16]) -> Vec<u8> {
    let to_be_signed = to_be_signed(key, name, serial);
    let signature = key.sign(&to_be_signed);
    sequence(&[&to_be_signed, &algorithm(), &bit_string(signature.as_ref())])
}

/// The part of the certificate its signature covers.
fn to_be_signed(key: &Ed25519KeyPair, name: &str, serial: [u8; 16]) -> Vec<u8> {
    let name_der = distinguished_name(name);
    let alt_names = sequence(&[&der(DNS_NAME, name.as_bytes())]);
    let extension = sequence(&[&der(OBJECT_IDENTIFIER, SUBJECT_ALT_NAME), &der(OCTET_STRING, &alt_names)]);
    sequence(&[
        &der(VERSION, &der(INTEGER, &[V3])),
        &der(INTEGER, &positive(serial)),
        &algorithm(),
        &name_der,
        &sequence(&[&der(UTC_TIME, NOT_BEFORE), &der(GENERALIZED_TIME, NOT_AFTER)]),
        &name_der,
        &sequence(&[&algorithm(), &bit_string(key.public_key().as_ref())]),
        &der(EXTENSIONS, &sequence(&[&extension])),
    ])
}

/// Ed25519's algorithm identifier, which has no parameters.
fn algorithm() -> Vec<u8> {
    sequence(&[&der(OBJECT_IDENTIFIER, ED25519)])
}

/// `serial` as a positive integer of as many bytes, which DER writes without a leading zero:
/// its top bit cleared, and the one below it set.
fn positive(mut serial: [u8; 16]) -> [u8; 16] {
    serial[0] = serial[0] & 0x7f | 0x40;
    serial
}

#[cfg(test)]
mod tests {
    use ring::signature::UnparsedPublicKey;
    use rustls::pki_types::ServerName;

    use super::*;

    fn key() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    #[test]
    fn lengths_take_the_short_form_below_128_and_the_long_form_from_it() {
        assert_eq!(der(OCTET_STRING, &[7; 127])[..2], [OCTET_STRING, 127]);
        assert_eq!(der(OCTET_STRING, &[7; 128])[..3], [OCTET_STRING, 0x81, 128]);
        assert_eq!(der(OCTET_STRING, &[7; 300])[..4], [OCTET_STRING, 0x82, 0x01, 0x2c]);
    }

    #[test]
    fn the_certificate_is_signed_by_its_key() {
        let (key, serial) = (key(), [0xff; 16]);
        let certificate = self_signed(&key, "server", serial);
        let to_be_signed = to_be_signed(&key, "server", serial);
        // Ed25519 signs deterministically, so the signature is the one in the certificate.
        let signature = key.sign(&to_be_signed);
        assert_eq!(certificate, sequence(&[&to_be_signed, &algorithm(), &bit_string(signature.as_ref())]));
        UnparsedPublicKey::new(&ring::signature::ED25519, key.public_key().as_ref()).verify(&to_be_signed, signature.as_ref()).unwrap();
    }

    #[test]
    fn tls_takes_the_certificate_for_the_server_s_name_and_key() {
        let key = key();
        let certificate = CertificateDer::from(self_signed(&key, "server", [0; 16]));
        let parsed = webpki::EndEntityCert::try_from(&certificate).unwrap();
        parsed.verify_is_valid_for_subject_name(&ServerName::try_from("server").unwrap()).unwrap();
        let message = b"the handshake";
        parsed.verify_signature(webpki::ring::ED25519, message, key.sign(message).as_ref()).unwrap();
    }
}
