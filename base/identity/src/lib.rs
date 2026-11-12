// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The identity an appliance makes for itself, once, at its first IPL
//! after installation: an Ed25519 key pair and a self-signed X.509
//! certificate that names the host and carries its serial number. A
//! management console later pairs with the host by this certificate.
//!
//! Two files in the state directory, both PEM: the private key
//! (PKCS #8, readable by root only) and the certificate. When both are
//! there nothing is made: the identity does not change with the host
//! name or the clock.
//!
//! The certificate is written here, field by field: it is some eighty
//! bytes of DER around a key and a signature.

#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use appliance::time::civil;
use ed25519_dalek::{Signer, SigningKey};

/// The private key's file in the state directory.
pub const KEY_FILE: &str = "identity.key";
/// The certificate's file in the state directory.
pub const CERT_FILE: &str = "identity.crt";

/// DER tags.
const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const INTEGER: u8 = 0x02;
const BIT_STRING: u8 = 0x03;
const OCTET_STRING: u8 = 0x04;
const OID: u8 = 0x06;
const UTF8_STRING: u8 = 0x0c;
const PRINTABLE_STRING: u8 = 0x13;
const UTC_TIME: u8 = 0x17;
const GENERALIZED_TIME: u8 = 0x18;
const BOOLEAN: u8 = 0x01;

/// `id-Ed25519`, 1.3.101.112.
const ED25519: [u8; 3] = [0x2b, 0x65, 0x70];
/// `commonName`, 2.5.4.3.
const COMMON_NAME: [u8; 3] = [0x55, 0x04, 0x03];
/// `serialNumber`, 2.5.4.5.
const SERIAL_NUMBER: [u8; 3] = [0x55, 0x04, 0x05];
/// `basicConstraints`, 2.5.29.19.
const BASIC_CONSTRAINTS: [u8; 3] = [0x55, 0x1d, 0x13];
/// `keyUsage`, 2.5.29.15.
const KEY_USAGE: [u8; 3] = [0x55, 0x1d, 0x0f];

/// One DER element: its tag, its length and `content`.
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(u8::try_from(len).unwrap_or(0));
    } else {
        let bytes = len.to_be_bytes();
        let skip = bytes.iter().take_while(|b| **b == 0).count();
        out.push(0x80 | u8::try_from(bytes.len() - skip).unwrap_or(0));
        out.extend_from_slice(&bytes[skip..]);
    }
    out.extend_from_slice(content);
    out
}

fn sequence(parts: &[Vec<u8>]) -> Vec<u8> {
    der(SEQUENCE, &parts.concat())
}

/// A bit string of whole bytes.
fn bits(bytes: &[u8]) -> Vec<u8> {
    der(BIT_STRING, &[&[0u8][..], bytes].concat())
}

fn algorithm() -> Vec<u8> {
    sequence(&[der(OID, &ED25519)])
}

/// One attribute of a name, in a set of its own.
fn attribute(oid: &[u8], tag: u8, value: &str) -> Vec<u8> {
    der(SET, &sequence(&[der(OID, oid), der(tag, value.as_bytes())]))
}

/// A serial number as a `PrintableString` may hold it.
fn printable(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || " '()+,-./:=?".contains(*c))
        .collect()
}

/// The name of the host: its common name, and its serial number when
/// it has one.
fn name(host: &str, serial: &str) -> Vec<u8> {
    let mut parts = vec![attribute(&COMMON_NAME, UTF8_STRING, host)];
    let serial = printable(serial);
    if !serial.is_empty() {
        parts.push(attribute(&SERIAL_NUMBER, PRINTABLE_STRING, &serial));
    }
    sequence(&parts)
}

/// A time as a certificate has it: `UTCTime` before 2050,
/// `GeneralizedTime` from then on.
fn time(secs: i64) -> Vec<u8> {
    let c = civil(secs);
    let rest = format!(
        "{:02}{:02}{:02}{:02}{:02}Z",
        c.month, c.day, c.hour, c.minute, c.second
    );
    if c.year < 2050 {
        der(UTC_TIME, format!("{:02}{rest}", c.year % 100).as_bytes())
    } else {
        der(GENERALIZED_TIME, format!("{:04}{rest}", c.year).as_bytes())
    }
}

/// An extension that is critical.
fn extension(oid: &[u8], value: &[u8]) -> Vec<u8> {
    sequence(&[
        der(OID, oid),
        der(BOOLEAN, &[0xff]),
        der(OCTET_STRING, value),
    ])
}

/// The certificate of `key` for the host `host` with the serial number
/// `serial`: self-signed, valid from `now` with no end, numbered
/// `number`.
pub fn certificate(
    key: &SigningKey,
    host: &str,
    serial: &str,
    now: i64,
    number: [u8; 16],
) -> Vec<u8> {
    let name = name(host, serial);
    // A positive integer: the first bit is clear.
    let mut number = number;
    number[0] = number[0] & 0x7f | 0x40;
    // No well-defined end, as RFC 5280 writes it.
    let never = der(GENERALIZED_TIME, b"99991231235959Z");
    let extensions = sequence(&[
        // Not a certificate authority.
        extension(&BASIC_CONSTRAINTS, &sequence(&[])),
        // digitalSignature.
        extension(&KEY_USAGE, &der(BIT_STRING, &[0x07, 0x80])),
    ]);
    let tbs = sequence(&[
        // Version 3.
        der(0xa0, &der(INTEGER, &[2])),
        der(INTEGER, &number),
        algorithm(),
        name.clone(),
        sequence(&[time(now), never]),
        name,
        sequence(&[algorithm(), bits(key.verifying_key().as_bytes())]),
        der(0xa3, &extensions),
    ]);
    let signature = key.sign(&tbs);
    sequence(&[tbs, algorithm(), bits(&signature.to_bytes())])
}

/// The private key as PKCS #8.
fn pkcs8(key: &SigningKey) -> Vec<u8> {
    sequence(&[
        der(INTEGER, &[0]),
        algorithm(),
        der(OCTET_STRING, &der(OCTET_STRING, &key.to_bytes())),
    ])
}

/// Base64, in lines of 64.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(3).enumerate() {
        if i > 0 && i % 16 == 0 {
            out.push('\n');
        }
        let n = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (j, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if j <= chunk.len() {
                out.push(char::from(TABLE[(n >> shift & 0x3f) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `der` as a PEM document of kind `label`.
pub fn pem(label: &str, der: &[u8]) -> String {
    format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        base64(der)
    )
}

fn random<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Write `text` to `path` for its owner alone, whole or not at all.
fn write_private(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("new");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        File::open(dir)?.sync_all()?;
    }
    Ok(())
}

/// Make the identity in `dir` unless it is there: whether it was made.
/// The certificate goes last, so a key without one is made again.
pub fn ensure(dir: &Path, host: &str, serial: &str, now: i64) -> io::Result<bool> {
    let (key_path, cert_path) = (dir.join(KEY_FILE), dir.join(CERT_FILE));
    if key_path.is_file() && cert_path.is_file() {
        return Ok(false);
    }
    fs::create_dir_all(dir)?;
    let key = SigningKey::from_bytes(&random::<32>()?);
    let cert = certificate(&key, host, serial, now, random::<16>()?);
    write_private(&key_path, &pem("PRIVATE KEY", &pkcs8(&key)))?;
    write_private(&cert_path, &pem("CERTIFICATE", &cert))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};

    /// The content of the element at the start of `bytes`, and what follows it.
    fn open(bytes: &[u8]) -> (u8, &[u8], &[u8]) {
        let (len, head) = if bytes[1] < 0x80 {
            (usize::from(bytes[1]), 2)
        } else {
            let n = usize::from(bytes[1] & 0x7f);
            let len = bytes[2..2 + n]
                .iter()
                .fold(0usize, |acc, b| acc << 8 | usize::from(*b));
            (len, 2 + n)
        };
        (bytes[0], &bytes[head..head + len], &bytes[head + len..])
    }

    #[test]
    fn the_certificate_is_signed_by_its_own_key_over_its_first_part() {
        let key = SigningKey::from_bytes(&[7; 32]);
        // 2026-11-12 13:05:00 UTC.
        let cert = certificate(&key, "vs-01", "M525400123456", 1_794_488_700, [0xff; 16]);
        let (tag, body, rest) = open(&cert);
        assert_eq!((tag, rest.len()), (SEQUENCE, 0));
        // The to-be-signed part, the algorithm, the signature.
        let tbs_len = body.len() - open(body).2.len();
        let tbs = &body[..tbs_len];
        let (_, _, after_tbs) = open(body);
        let (_, alg, after_alg) = open(after_tbs);
        assert_eq!(alg, der(OID, &ED25519));
        let (tag, signature, end) = open(after_alg);
        assert_eq!((tag, signature[0], end.len()), (BIT_STRING, 0, 0));
        let signature = Signature::from_slice(&signature[1..]).unwrap();
        key.verifying_key().verify(tbs, &signature).unwrap();

        // Version 3, a positive serial, the host's name twice, the key.
        let text = String::from_utf8_lossy(&cert);
        assert_eq!(text.matches("vs-01").count(), 2);
        assert_eq!(text.matches("M525400123456").count(), 2);
        assert!(text.contains("261112130500Z") && text.contains("99991231235959Z"));
        let (_, tbs_body, _) = open(tbs);
        assert_eq!(&tbs_body[..5], [0xa0, 3, INTEGER, 1, 2]);
        assert_eq!(tbs_body[7], 0x7f);
        let public = key.verifying_key();
        assert!(cert.windows(32).any(|w| w == public.as_bytes()));
    }

    #[test]
    fn long_lengths_dates_past_2049_and_pem_are_well_formed() {
        assert_eq!(der(OCTET_STRING, &[1; 3]), [4, 3, 1, 1, 1]);
        assert_eq!(&der(OCTET_STRING, &[0; 200])[..3], [4, 0x81, 200]);
        assert_eq!(&der(OCTET_STRING, &[0; 300])[..4], [4, 0x82, 1, 44]);
        // 2050-01-01 00:00:00 UTC.
        assert_eq!(
            time(2_524_608_000),
            der(GENERALIZED_TIME, b"20500101000000Z")
        );
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        let long = base64(&[0u8; 60]);
        assert_eq!(long.lines().map(str::len).collect::<Vec<_>>(), [64, 16]);
        let doc = pem("CERTIFICATE", b"Man");
        assert_eq!(
            doc,
            "-----BEGIN CERTIFICATE-----\nTWFu\n-----END CERTIFICATE-----\n"
        );
        assert_eq!(printable("AB_12*x"), "AB12x");
    }

    #[test]
    fn the_identity_is_made_once() {
        let dir = std::env::temp_dir().join(format!("identity-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(ensure(&dir, "vs-01", "", 1_794_488_700).unwrap());
        let key = fs::read_to_string(dir.join(KEY_FILE)).unwrap();
        let cert = fs::read_to_string(dir.join(CERT_FILE)).unwrap();
        assert!(key.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert!(cert.starts_with("-----BEGIN CERTIFICATE-----\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A second IPL, another name: the same identity.
        assert!(!ensure(&dir, "other", "", 1_800_000_000).unwrap());
        assert_eq!(fs::read_to_string(dir.join(CERT_FILE)).unwrap(), cert);
        // A key whose certificate was never written is made again.
        fs::remove_file(dir.join(CERT_FILE)).unwrap();
        assert!(ensure(&dir, "vs-01", "", 1_794_488_700).unwrap());
        assert_ne!(fs::read_to_string(dir.join(KEY_FILE)).unwrap(), key);
        fs::remove_dir_all(&dir).unwrap();
    }
}
