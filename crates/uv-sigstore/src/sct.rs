//! Verify the embedded RFC 6962 timestamp before using a certificate issued by Fulcio.
use jiff::Timestamp;
use ring::signature::{ECDSA_P256_SHA256_ASN1, UnparsedPublicKey};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use x509_cert::der::{Decode, Encode, asn1::ObjectIdentifier};
use x509_parser::extensions::{CtVersion, ParsedExtension};
use x509_parser::{
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
    #[error("Missing or invalid Fulcio issuer certificate")]
    Issuer,
    #[error("Missing or malformed signed certificate timestamp")]
    Missing,
    #[error("No certificate timestamp was signed by a trusted CT log")]
    Signature,
}

/// Reconstruct the signature input for an embedded SCT (RFC 6962 section 3.2).
pub(super) fn signature_input(
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
    issuer: &[u8],
) -> Result<(), TimestampError> {
    let Ok(([], issuer)) = parse_x509_certificate(issuer) else {
        return Err(TimestampError::Issuer);
    };
    let extension = cert
        .get_extension_unique(&OID_CT_LIST_SCT)
        .ok()
        .flatten()
        .ok_or(TimestampError::Missing)?;
    let ParsedExtension::SCT(timestamps) = extension.parsed_extension() else {
        return Err(TimestampError::Missing);
    };
    // The SCT signs the to-be-signed certificate before Fulcio inserts the SCT list itself.
    let mut tbs = x509_cert::TbsCertificate::from_der(cert.tbs_certificate.as_ref())?;
    tbs.extensions
        .as_mut()
        .ok_or(TimestampError::Missing)?
        .retain(|ext| ext.extn_id != SCT_LIST);
    let tbs = tbs.to_der()?;
    for sct in timestamps {
        let signature = &sct.signature;
        if sct.version != CtVersion::V1 || signature.hash_alg_id != 4 || signature.sign_alg_id != 3
        {
            continue; // Only RFC 6962 v1 with SHA-256 and ECDSA.
        }
        let Some(time) = i64::try_from(sct.timestamp)
            .ok()
            .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        else {
            continue;
        };
        let Some(key) = root.ct_key(sct.id.key_id, time) else {
            continue;
        };
        let Ok(([], spki)) = SubjectPublicKeyInfo::from_der(key.as_ref()) else {
            continue;
        };
        let input = signature_input(
            sct.timestamp,
            issuer.public_key().raw,
            &tbs,
            sct.extensions.0,
        )
        .ok_or(TimestampError::Missing)?;
        if UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, &*spki.subject_public_key.data)
            .verify(&input, signature.data)
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(TimestampError::Signature)
}
