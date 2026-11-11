//! Sign in-toto statements with Fulcio and record their DSSE envelopes in Rekor.
//!
//! Fulcio binds the CI identity to an ephemeral public key. The corresponding private key
//! signs the exact statement bytes, and Rekor records the signature and the certificate.
//! Its signed integration time lets a verifier establish when the short-lived certificate
//! was used, even after it expires.

mod rekor;

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
use rustls_pki_types::{CertificateDer, SubjectPublicKeyInfoDer, pem::PemObject as _};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use uv_macros::DeserializeUntagged;
use x509_parser::{error::X509Error, nom, parse_x509_certificate};

const FULCIO: &str = "https://fulcio.sigstore.dev/api/v2/signingCert";
// PyPI verifies Rekor v1 entries with an integrated timestamp.
// See <https://github.com/pypi/pypi-attestations/issues/147>.
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
    #[error("Fulcio returned an empty certificate chain")]
    EmptyCertificateChain,
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
/// The signer checks that Fulcio issued a certificate for its key and that Rekor recorded its
/// envelope. Verifiers must validate the Fulcio chain, signed certificate timestamp, and signing
/// identity against their trust policy.
pub async fn sign_statement(
    client: &ClientWithMiddleware,
    token: &str,
    statement: &str,
) -> Result<SignedStatement, SigningError> {
    let log_key = rekor::public_key()?;
    sign_with_services(client, FULCIO, REKOR, &log_key, token, statement).await
}

async fn sign_with_services(
    client: &ClientWithMiddleware,
    fulcio: &str,
    rekor: &str,
    log_key: &SubjectPublicKeyInfoDer<'_>,
    token: &str,
    statement: &str,
) -> Result<SignedStatement, SigningError> {
    check_identity_token(token)?;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(SigningError::GenerateKey)?;
    let pem = request_certificate(client, fulcio, token, &key).await?;
    let der = CertificateDer::from_pem_slice(pem.as_bytes())?;
    let (remaining, cert) = parse_x509_certificate(&der)?;
    if !remaining.is_empty() {
        return Err(SigningError::TrailingCertificateData);
    }
    if cert.public_key().raw != key.subject_public_key_info() {
        return Err(SigningError::KeyMismatch);
    }

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
        client, rekor, log_key, &envelope, statement, &signature, &pem,
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
) -> Result<String, SigningError> {
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
    response
        .signed_certificate_embedded_sct
        .chain
        .certificates
        .into_iter()
        .next()
        .ok_or(SigningError::EmptyCertificateChain)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::{Arc, OnceLock};

    use base64::Engine as _;
    use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
    use rcgen::{
        CertificateParams, CertificateSigningRequestParams, Issuer, KeyPair, PublicKeyData as _,
        SigningKey as _,
    };
    use reqwest::Client;
    use reqwest_middleware::{ClientBuilder, RequestBuilder};
    use serde_json::{Value, json};
    use sha2::{Digest as _, Sha256};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    use x509_parser::{asn1_rs::BitString, parse_x509_certificate, verify::verify_signature};

    use super::{request_certificate, sign_with_services};

    #[tokio::test]
    async fn sign_in_toto_statement() -> Result<(), Box<dyn Error>> {
        let server = MockServer::start().await;
        let issuer = Issuer::new(CertificateParams::default(), KeyPair::generate()?);
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
                let csr = CertificateSigningRequestParams::from_pem(
                    std::str::from_utf8(&csr).expect("PEM CSR"),
                )
                .expect("Valid CSR");
                let cert = csr.signed_by(&issuer).expect("Issue the test certificate");
                ResponseTemplate::new(200).set_body_json(json!({
                    "signedCertificateEmbeddedSct": {"chain": {"certificates": [cert.pem()]}}
                }))
            })
            .expect(1)
            .mount(&server)
            .await;
        let log_key = KeyPair::generate()?;
        let public_log_key = log_key.subject_public_key_info();
        let log_id = Sha256::digest(&public_log_key);
        let receipt = Arc::new(OnceLock::new());
        let mock_receipt = Arc::clone(&receipt);
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
                    "signedEntryTimestamp": BASE64_STANDARD.encode(timestamp),
                    "inclusionProof": {
                        "rootHash": hex::encode(root), "logIndex": 0, "treeSize": 2,
                        "hashes": [hex::encode(sibling)], "checkpoint": checkpoint,
                    }
                });
                mock_receipt.set(entry.clone()).expect("A single log entry");
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
            &public_log_key.into(),
            &token,
            &statement,
        )
        .await?;

        // Check the generated fields against the wire receipt before redacting them.
        let wire = receipt.get().expect("Rekor responded");
        let proof = &wire["verification"]["inclusionProof"];
        let root = hex::decode(proof["rootHash"].as_str().expect("Hex root hash"))?;
        let mut entry = signed.transparency_entry;
        for (field, expected) in [
            ("/canonicalizedBody", wire["body"].clone()),
            ("/logId/keyId", json!(BASE64_STANDARD.encode(log_id))),
            (
                "/inclusionProof/rootHash",
                json!(BASE64_STANDARD.encode(root)),
            ),
            (
                "/inclusionProof/checkpoint/envelope",
                proof["checkpoint"].clone(),
            ),
            (
                "/inclusionPromise/signedEntryTimestamp",
                wire["verification"]["signedEntryTimestamp"].clone(),
            ),
        ] {
            let actual = entry.pointer_mut(field).expect("Required receipt field");
            assert_eq!(*actual, expected, "{field}");
            *actual = json!("[generated]");
        }
        insta::assert_json_snapshot!(entry, @r#"
        {
          "canonicalizedBody": "[generated]",
          "inclusionPromise": {
            "signedEntryTimestamp": "[generated]"
          },
          "inclusionProof": {
            "checkpoint": {
              "envelope": "[generated]"
            },
            "hashes": [
              "q6urq6urq6urq6urq6urq6urq6urq6urq6urq6urq6s="
            ],
            "logIndex": "0",
            "rootHash": "[generated]",
            "treeSize": "2"
          },
          "integratedTime": "1700000000",
          "kindVersion": {
            "kind": "dsse",
            "version": "0.0.1"
          },
          "logId": {
            "keyId": "[generated]"
          },
          "logIndex": "10"
        }
        "#);

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
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403).set_body_string("issuer rejected token-secret"),
            )
            .mount(&server)
            .await;
        let error = request_certificate(
            &Client::new().into(),
            &server.uri(),
            "token-secret",
            &KeyPair::generate()?,
        )
        .await
        .expect_err("Fulcio rejected the request");
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
