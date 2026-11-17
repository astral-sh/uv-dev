//! Sign in-toto statements with Fulcio and record their DSSE envelopes in Rekor.
//!
//! Fulcio binds the CI identity to an ephemeral public key. The corresponding private key
//! signs the exact statement bytes, and Rekor records the signature and the certificate.
//! Its signed integration time lets a verifier establish when the short-lived certificate
//! was used, even after it expires.

mod rekor;
mod sct;
mod trust;

use base64::Engine as _;
use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
use itertools::Itertools as _;
use jiff::Timestamp;
use rcgen::{
    CertificateParams, DistinguishedName, KeyPair, PKCS_ECDSA_P256_SHA256, PublicKeyData as _,
    SigningKey as _,
};
pub use rekor::RekorError;
use reqwest::{Response, StatusCode};
use reqwest_middleware::ClientWithMiddleware;
use rustls_pki_types::{CertificateDer, pem::PemObject as _};
pub use sct::TimestampError;
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
pub use trust::{TrustError, TrustedRoot};
use uv_macros::DeserializeUntagged;
use x509_parser::{error::X509Error, nom, parse_x509_certificate};

const FULCIO: &str = "https://fulcio.sigstore.dev/api/v2/signingCert";
// PyPI verifies Rekor v1 entries with an integrated timestamp.
// See <https://github.com/pypi/pypi-attestations/issues/147>.
const REKOR_ORIGIN: &str = "https://rekor.sigstore.dev";
const REKOR: &str = "https://rekor.sigstore.dev/api/v1/log/entries";
const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

/// An unsuccessful Sigstore response, including the diagnostic supplied by the service.
#[derive(Debug, Error)]
#[error("{service} returned HTTP {status}{detail}")]
pub struct ServiceError {
    service: &'static str,
    status: StatusCode,
    detail: String,
    #[source]
    source: Option<reqwest::Error>,
}

impl ServiceError {
    async fn from_response(
        service: &'static str,
        response: Response,
        bearer_token: Option<&str>,
    ) -> Self {
        let status = response.status();
        let source = response.error_for_status_ref().err();
        let body = response.text().await.unwrap_or_default();
        let message = body.trim();
        let detail = if message.is_empty() {
            String::new()
        } else if let Some(token) = bearer_token {
            // Fulcio may echo request details when rejecting a credential.
            format!(": {}", message.replace(token, "[REDACTED]"))
        } else {
            format!(": {message}")
        };
        Self {
            service,
            status,
            detail,
            source,
        }
    }
}

/// Failure to sign a statement or obtain its verification material.
#[derive(Debug, Error)]
pub enum SigningError {
    #[error(transparent)]
    Trust(#[from] TrustError),
    #[error("Failed to verify Fulcio certificate transparency")]
    CertificateTimestamp(#[from] TimestampError),
    #[error("Invalid Sigstore identity token: expected three JWT segments")]
    TokenFormat,
    #[error("Invalid base64url payload in Sigstore identity token")]
    TokenEncoding(#[source] base64::DecodeError),
    #[error("Invalid OIDC claims at {line}:{column}: expected `aud` and a Unix timestamp in `exp`")]
    TokenClaims { line: usize, column: usize },
    #[error("The OIDC token must include `sigstore` in its audience")]
    TokenAudience,
    #[error("The Sigstore OIDC token expired at Unix time {0}")]
    TokenExpired(i64),
    #[error("Failed to generate the ephemeral signing key")]
    GenerateKey(#[source] rcgen::Error),
    #[error("Failed to construct the Fulcio certificate signing request")]
    CertificateRequest(#[source] rcgen::Error),
    #[error("Failed to obtain a signing certificate from Fulcio")]
    Fulcio(#[source] reqwest_middleware::Error),
    #[error(transparent)]
    FulcioStatus(ServiceError),
    #[error("Failed to decode the Fulcio certificate response")]
    FulcioResponse(#[source] reqwest::Error),
    #[error("Invalid Fulcio signing certificate PEM")]
    CertificatePem(#[from] rustls_pki_types::pem::Error),
    #[error("Invalid Fulcio signing certificate")]
    CertificateDer(#[from] nom::Err<X509Error>),
    #[error("Trailing bytes in the Fulcio signing certificate")]
    TrailingCertificateData,
    #[error("Fulcio returned a certificate for a different key")]
    KeyMismatch,
    #[error(
        "Rekor integration time {integrated_time} is outside the Fulcio certificate's validity \
         interval [{not_before}, {not_after}] (Unix seconds)"
    )]
    CertificateValidity {
        integrated_time: i64,
        not_before: i64,
        not_after: i64,
    },
    #[error("Failed to sign the DSSE envelope")]
    Sign(#[source] rcgen::Error),
    #[error("Failed to record the signed statement in Rekor")]
    Rekor(#[from] RekorError),
}

/// A signed in-toto statement and its Sigstore verification material.
pub struct SignedStatement {
    /// Base64-encoded bytes of the original statement.
    pub payload: String,
    /// Base64-encoded DER ECDSA signature over the DSSE pre-authentication encoding.
    pub signature: String,
    /// Base64-encoded DER of the Fulcio signing certificate.
    pub certificate: String,
    /// Rekor entry in Sigstore protobuf JSON format.
    pub transparency_entry: Value,
}

/// Sign a serialized in-toto statement using a CI identity token for the `sigstore` audience.
///
/// The signer checks that a trusted CT log witnessed a certificate for its ephemeral key and that
/// Rekor recorded the signed statement. Consumers still verify the Fulcio chain and identity.
pub async fn sign_statement(
    client: &ClientWithMiddleware,
    token: &str,
    statement: &str,
) -> Result<SignedStatement, SigningError> {
    TrustedRoot::fetch(client, None)
        .await?
        .sign_statement(client, token, statement)
        .await
}

impl TrustedRoot {
    /// Sign with the trust metadata already fetched for this publish session.
    pub async fn sign_statement(
        &self,
        client: &ClientWithMiddleware,
        token: &str,
        statement: &str,
    ) -> Result<SignedStatement, SigningError> {
        sign_with_services(client, FULCIO, REKOR, self, token, statement).await
    }
}

async fn sign_with_services(
    client: &ClientWithMiddleware,
    fulcio: &str,
    rekor: &str,
    trust: &TrustedRoot,
    token: &str,
    statement: &str,
) -> Result<SignedStatement, SigningError> {
    check_identity_token(token)?;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(SigningError::GenerateKey)?;
    let certificates = request_certificate(client, fulcio, token, &key).await?;
    let [pem, issuer, ..] = certificates.as_slice() else {
        return Err(TimestampError::Issuer.into());
    };
    let der = CertificateDer::from_pem_slice(pem.as_bytes())?;
    let (remaining, cert) = parse_x509_certificate(&der)?;
    if !remaining.is_empty() {
        return Err(SigningError::TrailingCertificateData);
    }
    if cert.public_key().raw != key.subject_public_key_info() {
        return Err(SigningError::KeyMismatch);
    }

    sct::verify(
        trust,
        &cert,
        &CertificateDer::from_pem_slice(issuer.as_bytes())?,
    )?;

    // DSSE signs the payload type and the exact payload bytes, each prefixed with its byte length.
    let pae = format!(
        "DSSEv1 {} {PAYLOAD_TYPE} {} {statement}",
        PAYLOAD_TYPE.len(),
        statement.len()
    );
    let signature = BASE64_STANDARD.encode(key.sign(pae.as_bytes()).map_err(SigningError::Sign)?);
    let payload = BASE64_STANDARD.encode(statement);
    // Rekor hashes the submitted envelope JSON verbatim. Reuse these bytes to check its receipt.
    let envelope = json!({
        "payloadType": PAYLOAD_TYPE,
        "payload": payload,
        "signatures": [{"sig": signature}],
    })
    .to_string();
    let (integrated_time, entry) = rekor::record(
        client,
        rekor,
        &trust.rekor,
        &envelope,
        statement,
        &signature,
        pem,
    )
    .await?;
    // Warehouse verifies the certificate at the time authenticated by Rekor. It may have
    // expired between issuance and log insertion, for example if a CI runner was suspended.
    let not_before = cert.validity().not_before.timestamp();
    let not_after = cert.validity().not_after.timestamp();
    if !(not_before..=not_after).contains(&integrated_time) {
        return Err(SigningError::CertificateValidity {
            integrated_time,
            not_before,
            not_after,
        });
    }
    Ok(SignedStatement {
        payload,
        signature,
        certificate: BASE64_STANDARD.encode(der),
        transparency_entry: entry,
    })
}

fn check_identity_token(token: &str) -> Result<(), SigningError> {
    #[derive(Deserialize)]
    struct Claims {
        aud: Audience,
        exp: i64,
    }

    #[derive(DeserializeUntagged)]
    #[serde(untagged)]
    enum Audience {
        #[untagged(string)]
        One(String),
        #[untagged(seq)]
        Many(Vec<String>),
    }

    // Check the intended audience before sending the token. Fulcio authenticates the identity.
    let (_, payload, _) = token
        .split('.')
        .collect_tuple()
        .ok_or(SigningError::TokenFormat)?;
    let payload = BASE64_URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(SigningError::TokenEncoding)?;
    // A Serde type error can quote a claim value. Report its location without logging token data.
    let claims: Claims =
        serde_json::from_slice(&payload).map_err(|err| SigningError::TokenClaims {
            line: err.line(),
            column: err.column(),
        })?;
    let for_sigstore = match claims.aud {
        Audience::One(audience) => audience == "sigstore",
        Audience::Many(audiences) => audiences.iter().any(|audience| audience == "sigstore"),
    };
    if !for_sigstore {
        return Err(SigningError::TokenAudience);
    }
    if claims.exp <= Timestamp::now().as_second() {
        return Err(SigningError::TokenExpired(claims.exp));
    }
    Ok(())
}

async fn request_certificate(
    client: &ClientWithMiddleware,
    url: &str,
    token: &str,
    key: &KeyPair,
) -> Result<Vec<String>, SigningError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct CertificateResponse {
        signed_certificate_embedded_sct: SignedCertificate,
    }
    #[derive(Deserialize)]
    struct SignedCertificate {
        chain: Chain,
    }
    #[derive(Deserialize)]
    struct Chain {
        certificates: Vec<String>,
    }

    // Fulcio derives the certificate identity from OIDC. The signed CSR proves possession
    // of the ephemeral private key without relying on an issuer-specific challenge claim.
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    let csr = params
        .serialize_request(key)
        .and_then(|request| request.pem())
        .map_err(SigningError::CertificateRequest)?;
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&json!({"certificateSigningRequest": BASE64_STANDARD.encode(csr)}))
        .send()
        .await
        .map_err(SigningError::Fulcio)?;
    if !response.status().is_success() {
        return Err(SigningError::FulcioStatus(
            ServiceError::from_response("Fulcio", response, Some(token)).await,
        ));
    }
    let response: CertificateResponse = response
        .json()
        .await
        .map_err(SigningError::FulcioResponse)?;
    Ok(response.signed_certificate_embedded_sct.chain.certificates)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::mpsc;

    use base64::Engine as _;
    use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
    use rcgen::{
        CertificateParams, CertificateSigningRequestParams, CertifiedIssuer, CustomExtension,
        KeyPair, KeyUsagePurpose, PublicKeyData as _, SigningKey as _,
    };
    use reqwest::Client;
    use reqwest_middleware::{ClientBuilder, RequestBuilder};
    use serde_json::{Value, json};
    use sha2::{Digest as _, Sha256};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    use x509_cert::der::{Encode as _, asn1::OctetString};
    use x509_parser::{asn1_rs::BitString, parse_x509_certificate, verify::verify_signature};

    use super::{ServiceError, sign_with_services};

    /// Act as Fulcio's CT log, signing the precertificate before Fulcio embeds the SCT list.
    fn timestamp_extension(
        ct_key: &KeyPair,
        issuer_spki: &[u8],
        precertificate: &[u8],
    ) -> Result<CustomExtension, Box<dyn Error>> {
        let timestamp = 1_700_000_000_000;
        let message = super::sct::signature_input(timestamp, issuer_spki, precertificate, &[])
            .ok_or("oversized precertificate")?;
        let signature = ct_key.sign(&message)?;
        let mut sct = vec![0]; // v1
        sct.extend(Sha256::digest(ct_key.subject_public_key_info()));
        sct.extend(timestamp.to_be_bytes());
        sct.extend([0, 0, 4, 3]); // Empty extensions, SHA-256, ECDSA.
        sct.extend(u16::try_from(signature.len())?.to_be_bytes());
        sct.extend(signature);

        // The extension is a DER octet string containing a TLS list of length-prefixed SCTs.
        let len = u16::try_from(sct.len())?;
        let mut list = (len + 2).to_be_bytes().to_vec();
        list.extend(len.to_be_bytes());
        list.extend(sct);
        let extension = OctetString::new(list)?.to_der()?;
        Ok(CustomExtension::from_oid_content(
            &[1, 3, 6, 1, 4, 1, 11129, 2, 4, 2],
            extension,
        ))
    }

    #[tokio::test]
    async fn sign_in_toto_statement() -> Result<(), Box<dyn Error>> {
        let server = MockServer::start().await;
        let issuer =
            CertifiedIssuer::self_signed(CertificateParams::default(), KeyPair::generate()?)?;
        let log_key = KeyPair::generate()?;
        let ct_key = KeyPair::generate()?;
        let log_id = Sha256::digest(log_key.subject_public_key_info());
        let log = |url: &str, key: &KeyPair| {
            json!({
                "baseUrl": url, "publicKey": {
                    "rawBytes": BASE64_STANDARD.encode(key.subject_public_key_info()),
                    "keyDetails": "PKIX_ECDSA_P256_SHA_256",
                    "validFor": {"start": "2000-01-01T00:00:00Z"}
                }
            })
        };
        let trust = json!({
            "mediaType": "application/vnd.dev.sigstore.trustedroot+json;version=0.1",
            "ctlogs": [log("https://ct.example", &ct_key)],
            "tlogs": [log(super::REKOR_ORIGIN, &log_key)],
        });
        let trust =
            super::TrustedRoot::parse(&serde_json::to_vec(&trust)?, jiff::Timestamp::now())?;
        let claims = json!({"aud": "sigstore", "exp": 4_102_444_800_i64});
        let claims = BASE64_URL_SAFE_NO_PAD.encode(claims.to_string());
        let token = format!("header.{claims}.signature");
        Mock::given(method("POST"))
            .and(header("x-test-client", "signer"))
            .and(path("/api/v2/signingCert"))
            .and(header("Authorization", format!("Bearer {token}")))
            .respond_with(move |request: &Request| {
                let request: Value = request.body_json().expect("Fulcio request");
                let csr = BASE64_STANDARD
                    .decode(
                        request["certificateSigningRequest"]
                            .as_str()
                            .expect("Certificate signing request"),
                    )
                    .expect("Base64 CSR");
                let mut csr = CertificateSigningRequestParams::from_pem(
                    std::str::from_utf8(&csr).expect("PEM CSR"),
                )
                .expect("Valid CSR");
                csr.params.serial_number = Some(1u64.into());
                csr.params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
                let precert = csr.signed_by(&issuer).expect("Issue precertificate");
                let (_, leaf) =
                    parse_x509_certificate(precert.der()).expect("Parse precertificate");
                let sct = timestamp_extension(
                    &ct_key,
                    &issuer.key().subject_public_key_info(),
                    leaf.tbs_certificate.as_ref(),
                )
                .expect("Issue test SCT");
                csr.params.custom_extensions.push(sct);
                let cert = csr.signed_by(&issuer).expect("Issue final certificate");
                ResponseTemplate::new(200).set_body_json(json!({
                    "signedCertificateEmbeddedSct": {
                        "chain": {"certificates": [cert.pem(), issuer.pem()]}
                    }
                }))
            })
            .expect(1)
            .mount(&server)
            .await;
        let (sender, receipt) = mpsc::channel();
        Mock::given(method("POST"))
            .and(header("x-test-client", "signer"))
            .and(path("/api/v1/log/entries"))
            .respond_with(move |request: &Request| {
                let request: Value = request.body_json().expect("Rekor request");
                let proposed = &request["spec"]["proposedContent"];
                let envelope = proposed["envelope"].as_str().expect("DSSE envelope");
                let parsed: Value = serde_json::from_str(envelope).expect("Valid envelope");
                let statement = BASE64_STANDARD
                    .decode(parsed["payload"].as_str().expect("Signed statement"))
                    .expect("Base64 statement");
                let envelope_hash = hex::encode(Sha256::digest(envelope));
                let payload_hash = hex::encode(Sha256::digest(statement));
                let body = serde_json::to_vec(&json!({
                    "apiVersion": "0.0.1", "kind": "dsse", "spec": {
                        "envelopeHash": {"algorithm": "sha256", "value": envelope_hash},
                        "payloadHash": {"algorithm": "sha256", "value": payload_hash},
                        "signatures": [{
                            "signature": parsed["signatures"][0]["sig"],
                            "verifier": proposed["verifiers"][0],
                        }],
                    }
                }))
                .expect("Canonical entry");
                let leaf = Sha256::new()
                    .chain_update([0])
                    .chain_update(&body)
                    .finalize();
                let sibling = [0xab; 32];
                let root = Sha256::new()
                    .chain_update([1])
                    .chain_update(leaf)
                    .chain_update(sibling)
                    .finalize();
                let root_b64 = BASE64_STANDARD.encode(root);
                let note = format!("test-log\n2\n{root_b64}\n");
                let mut signature = log_id[..4].to_vec();
                signature.extend(log_key.sign(note.as_bytes()).expect("Sign checkpoint"));
                let signature = BASE64_STANDARD.encode(signature);
                let checkpoint = format!("{note}\n— test-log {signature}\n");
                let mut entry = json!({
                    "body": BASE64_STANDARD.encode(&body), "integratedTime": 1_700_000_000,
                    "logID": hex::encode(log_id), "logIndex": 10,
                });
                let timestamp = log_key
                    .sign(entry.to_string().as_bytes())
                    .expect("Sign integration time");
                entry["verification"] = json!({
                    "signedEntryTimestamp": BASE64_STANDARD.encode(&timestamp),
                    "inclusionProof": {
                        "rootHash": hex::encode(root), "logIndex": 0, "treeSize": 2,
                        "hashes": [hex::encode(sibling)], "checkpoint": checkpoint,
                    }
                });
                sender
                    .send((body, root, timestamp, checkpoint))
                    .expect("Receipt receiver");
                ResponseTemplate::new(201).set_body_json(json!({"test-entry": entry}))
            })
            .expect(1)
            .mount(&server)
            .await;
        let statement = serde_json::to_string(&json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [],
            "predicateType": "https://example.com/test",
        }))?;
        let client = ClientBuilder::new(Client::new())
            .with_init(|request: RequestBuilder| request.header("x-test-client", "signer"))
            .build();
        let signed = sign_with_services(
            &client,
            &format!("{}/api/v2/signingCert", server.uri()),
            &format!("{}/api/v1/log/entries", server.uri()),
            &trust,
            &token,
            &statement,
        )
        .await?;

        // Use the values signed by the mock log to check even the generated fields exactly.
        let (body, root, timestamp, checkpoint) = receipt.try_recv()?;
        assert_eq!(
            signed.transparency_entry,
            json!({
                "canonicalizedBody": BASE64_STANDARD.encode(body),
                "inclusionPromise": {"signedEntryTimestamp": BASE64_STANDARD.encode(timestamp)},
                "inclusionProof": {
                    "checkpoint": {"envelope": checkpoint},
                    "hashes": ["q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s="],
                    "logIndex": "0",
                    "rootHash": BASE64_STANDARD.encode(root),
                    "treeSize": "2",
                },
                "integratedTime": "1700000000",
                "kindVersion": {"kind": "dsse", "version": "0.0.1"},
                "logId": {"keyId": BASE64_STANDARD.encode(log_id)},
                "logIndex": "10",
            })
        );

        assert_eq!(
            BASE64_STANDARD.decode(signed.payload)?,
            statement.as_bytes()
        );
        let der = BASE64_STANDARD.decode(signed.certificate)?;
        let (_, cert) = parse_x509_certificate(&der)?;
        let signature = BASE64_STANDARD.decode(signed.signature)?;
        let pae = format!(
            "DSSEv1 28 application/vnd.in-toto+json {} {statement}",
            statement.len()
        );
        verify_signature(
            cert.public_key(),
            &cert.signature_algorithm,
            &BitString::new(0, &signature),
            pae.as_bytes(),
        )?;
        Ok(())
    }

    #[tokio::test]
    async fn fulcio_rejection_retains_service_diagnostics() -> Result<(), Box<dyn Error>> {
        let response = http::Response::builder()
            .status(reqwest::StatusCode::FORBIDDEN)
            .body("issuer rejected token-secret")?;
        let error =
            ServiceError::from_response("Fulcio", response.into(), Some("token-secret")).await;
        insta::assert_snapshot!(error, @"Fulcio returned HTTP 403 Forbidden: issuer rejected [REDACTED]");
        let source = error
            .source()
            .and_then(|source| source.downcast_ref::<reqwest::Error>());
        assert_eq!(
            source.and_then(reqwest::Error::status),
            Some(reqwest::StatusCode::FORBIDDEN)
        );
        Ok(())
    }
}
