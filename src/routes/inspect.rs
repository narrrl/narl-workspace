//! `POST /api/nodes/{id}/inspect`: what a certificate, key or keystore file
//! holds, read on the server so the browser needs no ASN.1 of its own.
//!
//! PEM with any mix of blocks (a chain, a certificate with its key), DER
//! certificates, requests and keys, and PKCS#12 keystores — the legacy RC2 and
//! 3DES ones a keychain exports as well as AES. A password opens a keystore or
//! an encrypted key for this one answer; it is never stored and never logged.
//!
//! The answer describes keys — algorithm, size, which certificate each belongs
//! to — and never carries key material itself.

use std::io::Read;

use axum::{
    extract::{Path, State},
    Json,
};
use p12_keystore::{error::Error as P12Error, Pkcs12Archive};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use sqlx::Row;
use x509_parser::{
    certification_request::X509CertificationRequest,
    der_parser::der::parse_der_sequence,
    extensions::{GeneralName, ParsedExtension},
    pem::Pem,
    prelude::{FromDer, X509Certificate},
    public_key::{PublicKey, RSAPublicKey},
    x509::SubjectPublicKeyInfo,
};
use zeroize::Zeroizing;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    state::AppState,
    vault::{self, Password},
};

/// Certificates and keys are kilobytes. Anything past this is not one, and is
/// not worth reading into memory to find that out.
const MAX_INSPECT: u64 = 1024 * 1024;

#[derive(Deserialize, Default)]
pub struct InspectInput {
    #[serde(default)]
    pub password: Option<Password>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Item {
    Certificate {
        subject: String,
        issuer: String,
        serial: String,
        /// Unix seconds.
        not_before: i64,
        not_after: i64,
        /// `RSA 2048`, `EC P-256`, `Ed25519`.
        key: String,
        /// Subject alternative names.
        names: Vec<String>,
        /// Key usage and extended key usage, together.
        usage: Vec<String>,
        ca: bool,
        sha256: String,
        sha1: String,
        /// The friendly name a keystore gave it.
        label: Option<String>,
    },
    Request {
        subject: String,
        key: String,
        names: Vec<String>,
    },
    PrivateKey {
        /// None while the key is still encrypted.
        key: Option<String>,
        /// Whether it came under a password.
        encrypted: bool,
        /// The index of the certificate in `items` with the same public key.
        matches: Option<usize>,
        label: Option<String>,
    },
    PublicKey {
        key: String,
    },
}

#[derive(Serialize, Debug, Default)]
pub struct Inspection {
    pub items: Vec<Item>,
    /// Something here is behind a password that was not given.
    pub needs_password: bool,
}

#[derive(Debug, PartialEq)]
pub enum Failure {
    Unreadable,
    WrongPassword,
    /// Asks its key derivation for more work than any real export does.
    Unreasonable,
}

pub async fn inspect(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<InspectInput>,
) -> AppResult<Json<Inspection>> {
    let row = sqlx::query(
        "SELECT m.id, m.size, m.encrypted FROM nodes n JOIN media m ON m.id = n.media_id
         WHERE n.id = ?1 AND n.kind = 'file'",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let media_id: String = row.get("id");
    let size: i64 = row.get("size");
    let encrypted: bool = row.get("encrypted");

    if size.max(0) as u64 > MAX_INSPECT {
        return Err(AppError::BadRequest(
            "too large to be a certificate or a key".into(),
        ));
    }
    let key = match encrypted {
        true => Some(state.vault.key().ok_or(AppError::Locked)?),
        false => None,
    };
    let path = state.config.uploads_dir().join(&media_id);

    let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let mut bytes = Zeroizing::new(Vec::new());
        match key {
            Some(key) => {
                vault::Reader::open(&key, &media_id, &path)?
                    .take(MAX_INSPECT)
                    .read_to_end(&mut bytes)?;
            }
            None => {
                std::fs::File::open(&path)?
                    .take(MAX_INSPECT)
                    .read_to_end(&mut bytes)?;
            }
        }
        Ok(inspect_bytes(
            &bytes,
            input.password.as_ref().map(|p| p.as_str()),
        ))
    })
    .await
    .map_err(|e| AppError::Internal(e.into()))??;

    match outcome {
        Ok(inspection) => Ok(Json(inspection)),
        Err(Failure::WrongPassword) => Err(AppError::BadRequest("wrong password".into())),
        Err(Failure::Unreadable) => Err(AppError::BadRequest(
            "not a certificate, key or keystore this can read".into(),
        )),
        Err(Failure::Unreasonable) => Err(AppError::BadRequest(
            "this keystore asks for unreasonable key-stretching, and is not opened".into(),
        )),
    }
}

/// Everything the file holds, in the order it holds it.
pub fn inspect_bytes(bytes: &[u8], password: Option<&str>) -> Result<Inspection, Failure> {
    let mut found = Found::default();
    if bytes.windows(11).any(|w| w == b"-----BEGIN ") {
        for pem in Pem::iter_from_buffer(bytes).map_while(Result::ok) {
            found.pem(&pem, password)?;
        }
    } else {
        found.der(bytes, password)?;
    }
    if found.items.is_empty() && !found.needs_password {
        return Err(Failure::Unreadable);
    }
    Ok(found.finish())
}

/// The items so far, each with the public key it stands for where one is
/// known, so a private key can be paired with its certificate at the end.
#[derive(Default)]
struct Found {
    items: Vec<Item>,
    identities: Vec<Option<Vec<u8>>>,
    needs_password: bool,
}

impl Found {
    fn push(&mut self, item: Item, identity: Option<Vec<u8>>) {
        self.items.push(item);
        self.identities.push(identity);
    }

    fn finish(mut self) -> Inspection {
        let certificates: Vec<(usize, Vec<u8>)> = self
            .items
            .iter()
            .zip(&self.identities)
            .enumerate()
            .filter_map(|(at, (item, identity))| match (item, identity) {
                (Item::Certificate { .. }, Some(identity)) => Some((at, identity.clone())),
                _ => None,
            })
            .collect();
        for (item, identity) in self.items.iter_mut().zip(&self.identities) {
            if let (Item::PrivateKey { matches, .. }, Some(identity)) = (item, identity) {
                *matches = certificates
                    .iter()
                    .find(|(_, other)| other == identity)
                    .map(|(at, _)| *at);
            }
        }
        Inspection {
            items: self.items,
            needs_password: self.needs_password,
        }
    }

    fn pem(&mut self, pem: &Pem, password: Option<&str>) -> Result<(), Failure> {
        let der = &pem.contents;
        match pem.label.as_str() {
            "CERTIFICATE" | "X509 CERTIFICATE" | "TRUSTED CERTIFICATE" => {
                self.certificate(der, None);
            }
            "CERTIFICATE REQUEST" | "NEW CERTIFICATE REQUEST" => {
                self.request(der);
            }
            "PRIVATE KEY" => {
                self.private_key(der, false, None);
            }
            "ENCRYPTED PRIVATE KEY" => self.encrypted_key(der, password)?,
            "RSA PRIVATE KEY" => {
                // An old OpenSSL key under a password is this label with
                // headers; its body is then not an RSAPrivateKey at all.
                let (key, identity) = match rsa_modulus(der) {
                    Some(modulus) => (Some(rsa_name(modulus)), Some(trim(modulus).to_vec())),
                    None => (None, None),
                };
                let encrypted = key.is_none();
                self.push(
                    Item::PrivateKey {
                        key,
                        encrypted,
                        matches: None,
                        label: None,
                    },
                    identity,
                );
            }
            "EC PRIVATE KEY" => {
                if let Ok(ec) = sec1::EcPrivateKey::try_from(der.as_slice()) {
                    let curve = ec
                        .parameters
                        .and_then(|p| p.named_curve())
                        .map(|oid| curve_name(&oid.to_string()));
                    self.push(
                        Item::PrivateKey {
                            key: Some(ec_name(curve)),
                            encrypted: false,
                            matches: None,
                            label: None,
                        },
                        ec.public_key.map(<[u8]>::to_vec),
                    );
                }
            }
            "PUBLIC KEY" => {
                if let Ok((_, spki)) = SubjectPublicKeyInfo::from_der(der) {
                    let (key, identity) = describe(&spki);
                    self.push(Item::PublicKey { key }, identity);
                }
            }
            "RSA PUBLIC KEY" => {
                if let Ok((_, rsa)) = RSAPublicKey::from_der(der) {
                    self.push(
                        Item::PublicKey {
                            key: format!("RSA {}", rsa.key_size()),
                        },
                        Some(trim(rsa.modulus).to_vec()),
                    );
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// A DER file says nothing about what it is, so each reading is tried in
    /// turn, the strictest first.
    fn der(&mut self, der: &[u8], password: Option<&str>) -> Result<(), Failure> {
        if self.certificate(der, None) || self.request(der) {
            return Ok(());
        }
        if looks_like_pkcs12(der) {
            return self.pkcs12(der, password);
        }
        if self.private_key(der, false, None) {
            return Ok(());
        }
        if pkcs8::EncryptedPrivateKeyInfoRef::try_from(der).is_ok() {
            return self.encrypted_key(der, password);
        }
        if let Ok((_, spki)) = SubjectPublicKeyInfo::from_der(der) {
            let (key, identity) = describe(&spki);
            self.push(Item::PublicKey { key }, identity);
        }
        Ok(())
    }

    fn certificate(&mut self, der: &[u8], label: Option<String>) -> bool {
        let Ok((_, cert)) = X509Certificate::from_der(der) else {
            return false;
        };
        let (key, identity) = describe(cert.public_key());
        let names = cert
            .subject_alternative_name()
            .ok()
            .flatten()
            .map(|san| san.value.general_names.iter().map(name).collect())
            .unwrap_or_default();
        let mut usage: Vec<String> = cert
            .key_usage()
            .ok()
            .flatten()
            .map(|ku| ku.value.to_string())
            .into_iter()
            .flat_map(|list| list.split(", ").map(str::to_string).collect::<Vec<_>>())
            .filter(|s| !s.is_empty())
            .collect();
        if let Ok(Some(eku)) = cert.extended_key_usage() {
            let eku = eku.value;
            for (on, what) in [
                (eku.any, "Any Purpose"),
                (eku.server_auth, "Server Authentication"),
                (eku.client_auth, "Client Authentication"),
                (eku.code_signing, "Code Signing"),
                (eku.email_protection, "Email Protection"),
                (eku.time_stamping, "Time Stamping"),
                (eku.ocsp_signing, "OCSP Signing"),
            ] {
                if on {
                    usage.push(what.into());
                }
            }
            usage.extend(eku.other.iter().map(|oid| oid.to_id_string()));
        }
        let ca = cert
            .basic_constraints()
            .ok()
            .flatten()
            .is_some_and(|bc| bc.value.ca);
        self.push(
            Item::Certificate {
                subject: cert.subject().to_string(),
                issuer: cert.issuer().to_string(),
                serial: cert.raw_serial_as_string(),
                not_before: cert.validity().not_before.timestamp(),
                not_after: cert.validity().not_after.timestamp(),
                key,
                names,
                usage,
                ca,
                sha256: fingerprint(&Sha256::digest(der)),
                sha1: fingerprint(&Sha1::digest(der)),
                label,
            },
            identity,
        );
        true
    }

    fn request(&mut self, der: &[u8]) -> bool {
        let Ok((_, csr)) = X509CertificationRequest::from_der(der) else {
            return false;
        };
        let info = &csr.certification_request_info;
        let (key, _) = describe(&info.subject_pki);
        let names = csr
            .requested_extensions()
            .into_iter()
            .flatten()
            .filter_map(|ext| match ext {
                ParsedExtension::SubjectAlternativeName(san) => Some(san),
                _ => None,
            })
            .flat_map(|san| san.general_names.iter().map(name))
            .collect();
        self.push(
            Item::Request {
                subject: info.subject.to_string(),
                key,
                names,
            },
            None,
        );
        true
    }

    /// A PKCS#8 key, in the clear by now.
    fn private_key(&mut self, der: &[u8], encrypted: bool, label: Option<String>) -> bool {
        let Ok(info) = pkcs8::PrivateKeyInfoRef::try_from(der) else {
            return false;
        };
        let body = info.private_key.as_bytes();
        let (key, identity) = match info.algorithm.oid.to_string().as_str() {
            RSA => match rsa_modulus(body) {
                Some(modulus) => (rsa_name(modulus), Some(trim(modulus).to_vec())),
                None => ("RSA".into(), None),
            },
            EC => {
                let curve = info
                    .algorithm
                    .parameters
                    .and_then(|p| p.decode_as::<pkcs8::ObjectIdentifier>().ok())
                    .map(|oid| curve_name(&oid.to_string()));
                let public = sec1::EcPrivateKey::try_from(body)
                    .ok()
                    .and_then(|ec| ec.public_key.map(<[u8]>::to_vec));
                (ec_name(curve), public)
            }
            ED25519 => (
                "Ed25519".into(),
                info.public_key
                    .and_then(|k| k.as_bytes().map(<[u8]>::to_vec)),
            ),
            ED448 => ("Ed448".into(), None),
            other => (other.to_string(), None),
        };
        self.push(
            Item::PrivateKey {
                key: Some(key),
                encrypted,
                matches: None,
                label,
            },
            identity,
        );
        true
    }

    fn encrypted_key(&mut self, der: &[u8], password: Option<&str>) -> Result<(), Failure> {
        let Ok(info) = pkcs8::EncryptedPrivateKeyInfoRef::try_from(der) else {
            return Ok(());
        };
        let Some(password) = password else {
            self.needs_password = true;
            self.push(
                Item::PrivateKey {
                    key: None,
                    encrypted: true,
                    matches: None,
                    label: None,
                },
                None,
            );
            return Ok(());
        };
        reasonable_stretching(der)?;
        let plain = info.decrypt(password).map_err(|_| Failure::WrongPassword)?;
        if !self.private_key(plain.as_bytes(), true, None) {
            return Err(Failure::WrongPassword);
        }
        Ok(())
    }

    /// A keystore is tried with no password first, as plenty are exported
    /// with an empty one.
    fn pkcs12(&mut self, der: &[u8], password: Option<&str>) -> Result<(), Failure> {
        reasonable_stretching(der)?;
        let archive = match Pkcs12Archive::from_pkcs12(der, password.unwrap_or("")) {
            Ok(archive) => archive,
            Err(e) if password_problem(&e) => {
                if password.is_some() {
                    return Err(Failure::WrongPassword);
                }
                self.needs_password = true;
                return Ok(());
            }
            Err(e) => {
                tracing::debug!("unreadable keystore: {e}");
                return Err(Failure::Unreadable);
            }
        };
        for bag in &archive.certs {
            self.certificate(bag.cert.as_der(), bag.friendly_name.clone());
        }
        for bag in &archive.keys {
            let der = Zeroizing::new(bag.key.as_der().to_vec());
            self.private_key(&der, true, bag.friendly_name.clone());
        }
        Ok(())
    }
}

/// The most rounds a key derivation may ask for. Exports use from two
/// thousand to a few hundred thousand; ten million already takes seconds.
const MAX_ROUNDS: u64 = 10_000_000;
/// scrypt's cost and block size, at most: a gigabyte of memory between them.
const MAX_SCRYPT_COST: u64 = 1 << 20;
const MAX_SCRYPT_BLOCK: u64 = 8;
const SCRYPT: &str = "1.3.6.1.4.1.11591.4.11";

/// Refuse a key or keystore whose key derivation asks for more than any real
/// one does, before running it. The parameters are the file's to choose, and
/// a derivation runs to its end on a blocking thread whatever deadline the
/// request had — a crafted file opened in the browser would pin that thread
/// for hours, or ask scrypt for more memory than the machine has.
///
/// Every place a derivation takes its parameters from has the salt
/// followed by the count: PBKDF2's parameters, the PKCS#12 PBEs', the MAC's
/// `{ digest, salt, iterations }`, and scrypt's `{ salt, N, r, p }`. So any
/// sequence with an INTEGER after an OCTET STRING is held to the ceiling,
/// throughout the file, inside the octet strings the content sits in too.
fn reasonable_stretching(der: &[u8]) -> Result<(), Failure> {
    use x509_parser::der_parser::der::parse_der;
    match parse_der(der) {
        Ok((_, object)) => stretching_in(&object),
        Err(_) => Ok(()),
    }
}

fn stretching_in(object: &x509_parser::der_parser::ber::BerObject) -> Result<(), Failure> {
    use x509_parser::der_parser::{ber::BerObjectContent as Content, der::parse_der};

    let count = |object: &x509_parser::der_parser::ber::BerObject| {
        object.as_u64().map_err(|_| Failure::Unreasonable)
    };
    // What a constructed or wrapping value holds, when it is DER itself.
    let inner = |mut bytes: &[u8]| -> Result<(), Failure> {
        while let Ok((rest, object)) = parse_der(bytes) {
            stretching_in(&object)?;
            if rest.len() == bytes.len() {
                break;
            }
            bytes = rest;
        }
        Ok(())
    };
    match &object.content {
        Content::Sequence(items) | Content::Set(items) => {
            if let [algorithm, parameters] = items.as_slice() {
                let scrypt = algorithm
                    .as_oid_val()
                    .is_ok_and(|oid| oid.to_id_string() == SCRYPT);
                if scrypt {
                    let fields = parameters
                        .as_sequence()
                        .map_err(|_| Failure::Unreasonable)?;
                    let cost = fields.get(1).map(count).transpose()?.unwrap_or(0);
                    let block = fields.get(2).map(count).transpose()?.unwrap_or(0);
                    if cost > MAX_SCRYPT_COST || block > MAX_SCRYPT_BLOCK {
                        return Err(Failure::Unreasonable);
                    }
                }
            }
            for pair in items.windows(2) {
                if matches!(pair[0].content, Content::OctetString(_))
                    && matches!(pair[1].content, Content::Integer(_))
                    && count(&pair[1])? > MAX_ROUNDS
                {
                    return Err(Failure::Unreasonable);
                }
            }
            items.iter().try_for_each(stretching_in)
        }
        Content::OctetString(bytes) => inner(bytes),
        Content::Tagged(_, _, object) => stretching_in(object),
        Content::Unknown(any) => inner(any.data),
        _ => Ok(()),
    }
}

/// Whatever goes wrong once the structure has been recognised: a bad MAC, or
/// bags that decrypt to garbage.
fn password_problem(e: &P12Error) -> bool {
    !matches!(
        e,
        P12Error::InvalidVersion | P12Error::UnsupportedContentType | P12Error::IoError(_)
    )
}

/// `PFX ::= SEQUENCE { version INTEGER (3), authSafe ContentInfo, … }`, with
/// the ContentInfo of type `data` — enough to tell a keystore from any other
/// DER without its password.
fn looks_like_pkcs12(der: &[u8]) -> bool {
    let Ok((_, outer)) = parse_der_sequence(der) else {
        return false;
    };
    let Ok(fields) = outer.as_sequence() else {
        return false;
    };
    let version = fields.first().and_then(|v| v.as_u32().ok());
    let content_type = fields
        .get(1)
        .and_then(|info| info.as_sequence().ok())
        .and_then(|info| info.first())
        .and_then(|oid| oid.as_oid().ok())
        .map(|oid| oid.to_id_string());
    version == Some(3) && content_type.as_deref() == Some("1.2.840.113549.1.7.1")
}

const RSA: &str = "1.2.840.113549.1.1.1";
const EC: &str = "1.2.840.10045.2.1";
const ED25519: &str = "1.3.101.112";
const ED448: &str = "1.3.101.113";

/// What a public key is, and the bytes that make it that key.
fn describe(spki: &SubjectPublicKeyInfo) -> (String, Option<Vec<u8>>) {
    match spki.parsed() {
        Ok(PublicKey::RSA(rsa)) => (rsa_name(rsa.modulus), Some(trim(rsa.modulus).to_vec())),
        Ok(PublicKey::EC(point)) => {
            let curve = spki
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.as_oid().ok())
                .map(|oid| curve_name(&oid.to_id_string()));
            (ec_name(curve), Some(point.data().to_vec()))
        }
        _ => match spki.algorithm.algorithm.to_id_string().as_str() {
            ED25519 => (
                "Ed25519".into(),
                Some(spki.subject_public_key.data.to_vec()),
            ),
            ED448 => ("Ed448".into(), Some(spki.subject_public_key.data.to_vec())),
            other => (other.to_string(), None),
        },
    }
}

/// `RSAPrivateKey ::= SEQUENCE { version, modulus, … }`.
fn rsa_modulus(der: &[u8]) -> Option<&[u8]> {
    let (_, key) = parse_der_sequence(der).ok()?;
    let fields = key.as_sequence().ok()?;
    fields.get(1)?.as_slice().ok()
}

fn rsa_name(modulus: &[u8]) -> String {
    let modulus = trim(modulus);
    let bits = match modulus.first() {
        Some(top) => modulus.len() * 8 - top.leading_zeros() as usize,
        None => 0,
    };
    format!("RSA {bits}")
}

/// An unsigned integer as DER writes it may start with a zero byte.
fn trim(integer: &[u8]) -> &[u8] {
    let zeros = integer.iter().take_while(|b| **b == 0).count();
    &integer[zeros..]
}

fn curve_name(oid: &str) -> String {
    match oid {
        "1.2.840.10045.3.1.7" => "P-256".into(),
        "1.3.132.0.34" => "P-384".into(),
        "1.3.132.0.35" => "P-521".into(),
        "1.3.132.0.10" => "secp256k1".into(),
        other => other.into(),
    }
}

fn ec_name(curve: Option<String>) -> String {
    match curve {
        Some(curve) => format!("EC {curve}"),
        None => "EC".into(),
    }
}

fn name(general: &GeneralName) -> String {
    match general {
        GeneralName::DNSName(dns) => dns.to_string(),
        GeneralName::RFC822Name(mail) => mail.to_string(),
        GeneralName::URI(uri) => uri.to_string(),
        GeneralName::IPAddress(ip) => {
            if let Ok(v4) = <[u8; 4]>::try_from(*ip) {
                std::net::Ipv4Addr::from(v4).to_string()
            } else if let Ok(v6) = <[u8; 16]>::try_from(*ip) {
                std::net::Ipv6Addr::from(v6).to_string()
            } else {
                hex::encode(ip)
            }
        }
        other => other.to_string(),
    }
}

/// `AB:CD:…`, as every other tool prints one.
fn fingerprint(digest: &[u8]) -> String {
    digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Throwaway material, made once with OpenSSL 3 and nothing else signed by it:
    //   openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem \
    //     -days 3650 -subj "/CN=narl test/O=narl" -addext "subjectAltName=DNS:example.test"
    //   openssl x509 -in cert.pem -outform DER -out cert.der
    //   openssl req -new -key key.pem -out req.csr -subj "/CN=narl request"
    //   openssl pkcs8 -topk8 -in key.pem -out key.enc.pem -v2 aes-256-cbc -passout pass:test
    //   openssl pkcs12 -export -inkey key.pem -in cert.pem -out modern.p12 -passout pass:test
    //   openssl pkcs12 -export -legacy -inkey key.pem -in cert.pem -out legacy.p12 -passout pass:test
    //   openssl ecparam -name prime256v1 -genkey -noout | openssl pkcs8 -topk8 -nocrypt -out ec.pem
    //   cat cert.pem key.pem > bundle.pem
    const CERT_PEM: &[u8] = include_bytes!("testdata/cert.pem");
    const CERT_DER: &[u8] = include_bytes!("testdata/cert.der");
    const BUNDLE: &[u8] = include_bytes!("testdata/bundle.pem");
    const REQUEST: &[u8] = include_bytes!("testdata/req.csr");
    const ENCRYPTED_KEY: &[u8] = include_bytes!("testdata/key.enc.pem");
    const EC_KEY: &[u8] = include_bytes!("testdata/ec.pem");
    const MODERN: &[u8] = include_bytes!("testdata/modern.p12");
    const LEGACY: &[u8] = include_bytes!("testdata/legacy.p12");

    const SHA256: &str = "EA:C1:0E:4D:3C:41:C2:73:84:5E:7F:E0:7A:C2:39:9C:\
        0E:6A:D1:41:7D:BE:F6:DC:36:C7:56:66:E4:88:BD:75";

    fn certificate(item: &Item) -> (&str, &str, &[String], &str) {
        match item {
            Item::Certificate {
                subject,
                key,
                names,
                sha256,
                ..
            } => (subject, key, names, sha256),
            other => panic!("not a certificate: {other:?}"),
        }
    }

    #[test]
    fn reads_a_certificate_as_pem_and_as_der() {
        for bytes in [CERT_PEM, CERT_DER] {
            let found = inspect_bytes(bytes, None).unwrap();
            assert_eq!(found.items.len(), 1);
            let (subject, key, names, sha256) = certificate(&found.items[0]);
            assert!(subject.contains("CN=narl test"), "{subject}");
            assert_eq!(key, "RSA 2048");
            assert_eq!(names, ["example.test"]);
            assert_eq!(sha256, SHA256);
        }
    }

    #[test]
    fn pairs_a_key_with_its_certificate() {
        let found = inspect_bytes(BUNDLE, None).unwrap();
        assert_eq!(found.items.len(), 2);
        assert_eq!(
            found.items[1],
            Item::PrivateKey {
                key: Some("RSA 2048".into()),
                encrypted: false,
                matches: Some(0),
                label: None,
            }
        );
    }

    #[test]
    fn reads_a_request() {
        let found = inspect_bytes(REQUEST, None).unwrap();
        assert!(matches!(
            &found.items[..],
            [Item::Request { subject, key, .. }] if subject.contains("narl request") && key == "RSA 2048"
        ));
    }

    #[test]
    fn reads_an_ec_key() {
        let found = inspect_bytes(EC_KEY, None).unwrap();
        assert!(matches!(
            &found.items[..],
            [Item::PrivateKey { key: Some(key), encrypted: false, .. }] if key == "EC P-256"
        ));
    }

    #[test]
    fn an_encrypted_key_asks_for_its_password() {
        let found = inspect_bytes(ENCRYPTED_KEY, None).unwrap();
        assert!(found.needs_password);
        assert!(matches!(
            &found.items[..],
            [Item::PrivateKey {
                key: None,
                encrypted: true,
                ..
            }]
        ));

        assert_eq!(
            inspect_bytes(ENCRYPTED_KEY, Some("nope")).unwrap_err(),
            Failure::WrongPassword
        );

        let found = inspect_bytes(ENCRYPTED_KEY, Some("test")).unwrap();
        assert!(!found.needs_password);
        assert!(matches!(
            &found.items[..],
            [Item::PrivateKey { key: Some(key), encrypted: true, .. }] if key == "RSA 2048"
        ));
    }

    #[test]
    fn opens_modern_and_legacy_keystores() {
        for bytes in [MODERN, LEGACY] {
            let locked = inspect_bytes(bytes, None).unwrap();
            assert!(locked.needs_password);
            assert!(locked.items.is_empty());

            assert_eq!(
                inspect_bytes(bytes, Some("nope")).unwrap_err(),
                Failure::WrongPassword
            );

            let found = inspect_bytes(bytes, Some("test")).unwrap();
            assert_eq!(found.items.len(), 2, "{:?}", found.items);
            assert_eq!(certificate(&found.items[0]).3, SHA256);
            assert!(matches!(
                &found.items[1],
                Item::PrivateKey { key: Some(key), encrypted: true, matches: Some(0), .. } if key == "RSA 2048"
            ));
        }
    }

    #[test]
    fn never_answers_with_key_material() {
        let found = inspect_bytes(MODERN, Some("test")).unwrap();
        let json = serde_json::to_string(&found).unwrap();
        // The longest field anywhere is a fingerprint; a key would be kilobytes.
        assert!(json.len() < 2048, "{json}");
    }

    /// A count past the ceiling is refused wherever it sits, before any
    /// derivation runs; the counts real exports use pass.
    #[test]
    fn refuses_unreasonable_key_stretching() {
        let params = |count: &[u8]| {
            let mut der = vec![0x04, 0x02, 0xaa, 0xbb, 0x02, count.len() as u8];
            der.extend_from_slice(count);
            let mut seq = vec![0x30, der.len() as u8];
            seq.extend(der);
            seq
        };
        assert!(reasonable_stretching(&params(&[0x08, 0x00])).is_ok());
        assert!(matches!(
            reasonable_stretching(&params(&[0x7f, 0xff, 0xff, 0xff])),
            Err(Failure::Unreasonable)
        ));

        // Inside an octet string inside a sequence, as a keystore's bags are.
        let deep = params(&[0x7f, 0xff, 0xff, 0xff]);
        let mut wrapped = vec![0x04, deep.len() as u8];
        wrapped.extend(deep);
        let mut outer = vec![0x30, wrapped.len() as u8];
        outer.extend(wrapped);
        assert!(matches!(
            reasonable_stretching(&outer),
            Err(Failure::Unreasonable)
        ));

        // scrypt { salt, N = 2^22, r = 8, p = 1 }: four gigabytes.
        let oid = [
            0x06, 0x09, 0x2b, 0x06, 0x01, 0x04, 0x01, 0xda, 0x47, 0x04, 0x0b,
        ];
        let fields = [
            0x30, 0x0f, 0x04, 0x02, 0xaa, 0xbb, 0x02, 0x03, 0x40, 0x00, 0x00, 0x02, 0x01, 0x08,
            0x02, 0x01, 0x01,
        ];
        let mut scrypt = vec![0x30, (oid.len() + fields.len()) as u8];
        scrypt.extend(oid);
        scrypt.extend(fields);
        assert!(matches!(
            reasonable_stretching(&scrypt),
            Err(Failure::Unreasonable)
        ));

        // The fixtures are ordinary exports.
        for fixture in [MODERN, LEGACY] {
            assert!(reasonable_stretching(fixture).is_ok());
        }
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert_eq!(
            inspect_bytes(b"hello, world", None).unwrap_err(),
            Failure::Unreadable
        );
        assert_eq!(
            inspect_bytes(
                b"-----BEGIN NOTHING-----\naGk=\n-----END NOTHING-----\n",
                None
            )
            .unwrap_err(),
            Failure::Unreadable
        );
    }
}
