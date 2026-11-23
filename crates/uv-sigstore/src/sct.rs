//! Verify the embedded RFC 6962 timestamp before using a certificate issued by Fulcio.
use aws_lc_rs::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use jiff::Timestamp;
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use x509_cert::der::{Decode, Encode, asn1::ObjectIdentifier};
use x509_parser::extensions::{CtVersion, ParsedExtension};
use x509_parser::{
    error::X509Error,
    nom,
    oid_registry::OID_CT_LIST_SCT,
    parse_x509_certificate,
    prelude::{FromDer as _, SubjectPublicKeyInfo, X509Certificate},
};

use crate::TrustedRoot;

const SCT_LIST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.4.1.11129.2.4.2");

#[derive(Debug, Error)]
pub enum TimestampError {
    #[error("Invalid precertificate encoding")]
    Encoding(#[from] x509_cert::der::Error),
    #[error("Missing Fulcio issuer certificate")]
    MissingIssuer,
    #[error("Invalid Fulcio issuer certificate PEM")]
    IssuerPem(#[source] pem_rfc7468::Error),
    #[error("Invalid Fulcio issuer certificate")]
    IssuerDer(#[source] nom::Err<X509Error>),
    #[error("Unexpected trailing data after the Fulcio issuer certificate ({0} bytes)")]
    TrailingIssuerData(usize),
    #[error("Missing or malformed signed certificate timestamp")]
    Missing,
    #[error("Expected one signed certificate timestamp, got {0}")]
    Count(usize),
    #[error("No valid key for certificate transparency log {0}")]
    LogKey(String),
    #[error("Invalid certificate timestamp signature")]
    Signature,
}

/// Reconstruct the signature input for an embedded SCT (RFC 6962 section 3.2).
fn signature_input(
    timestamp: u64,
    issuer_spki: &[u8],
    tbs: &[u8],
    extensions: &[u8],
) -> Option<Vec<u8>> {
    let length = u32::try_from(tbs.len()).ok()?;
    if length > 0xff_ffff {
        return None;
    }
    let mut data = vec![0, 0]; // v1, certificate_timestamp
    data.extend(timestamp.to_be_bytes());
    data.extend(1u16.to_be_bytes()); // precert_entry
    data.extend(Sha256::digest(issuer_spki));
    data.extend(&length.to_be_bytes()[1..]);
    data.extend(tbs);
    data.extend(u16::try_from(extensions.len()).ok()?.to_be_bytes());
    data.extend(extensions);
    Some(data)
}

pub(super) fn verify(
    root: &TrustedRoot,
    cert: &X509Certificate<'_>,
    issuer_der: &[u8],
) -> Result<(), TimestampError> {
    let (remaining, issuer) =
        parse_x509_certificate(issuer_der).map_err(TimestampError::IssuerDer)?;
    if !remaining.is_empty() {
        return Err(TimestampError::TrailingIssuerData(remaining.len()));
    }
    let extension = cert
        .get_extension_unique(&OID_CT_LIST_SCT)
        .ok()
        .flatten()
        .ok_or(TimestampError::Missing)?;
    let ParsedExtension::SCT(timestamps) = extension.parsed_extension() else {
        return Err(TimestampError::Missing);
    };
    // PyPI's Sigstore verifier requires a single embedded timestamp.
    let [sct] = timestamps.as_slice() else {
        return Err(TimestampError::Count(timestamps.len()));
    };
    let signature = &sct.signature;
    // Only RFC 6962 v1 with SHA-256 and ECDSA.
    if sct.version != CtVersion::V1 || signature.hash_alg_id != 4 || signature.sign_alg_id != 3 {
        return Err(TimestampError::Signature);
    }
    let time = i64::try_from(sct.timestamp)
        .ok()
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .ok_or(TimestampError::Missing)?;
    let invalid_key = || TimestampError::LogKey(hex::encode(sct.id.key_id));
    let key = root.ct_key(sct.id.key_id, time).ok_or_else(invalid_key)?;
    let Ok(([], spki)) = SubjectPublicKeyInfo::from_der(key.as_ref()) else {
        return Err(invalid_key());
    };

    // The SCT signs the to-be-signed certificate before Fulcio inserts the SCT list itself.
    let mut tbs = x509_cert::TbsCertificate::from_der(cert.tbs_certificate.as_ref())?;
    tbs.extensions
        .as_mut()
        .ok_or(TimestampError::Missing)?
        .retain(|ext| ext.extn_id != SCT_LIST);
    let input = signature_input(
        sct.timestamp,
        issuer.public_key().raw,
        &tbs.to_der()?,
        sct.extensions.0,
    )
    .ok_or(TimestampError::Missing)?;
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &*spki.subject_public_key.data)
        .verify(&input, signature.data)
        .map_err(|_| TimestampError::Signature)
}
