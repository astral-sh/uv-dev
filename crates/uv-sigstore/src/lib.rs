//! Sign in-toto statements with Fulcio and record their DSSE envelopes in Rekor.

mod rekor;

use anyhow::{Context, Result, ensure};
use base64::Engine as _;
use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
use itertools::Itertools as _;
use jiff::Timestamp;
use rcgen::{
    CertificateParams, DistinguishedName, KeyPair, PKCS_ECDSA_P256_SHA256, PublicKeyData as _,
    SigningKey as _,
};
use reqwest::Client;
use rustls_pki_types::{CertificateDer, pem::PemObject as _};
use serde::Deserialize;
use serde_json::{Value, json};
use x509_parser::parse_x509_certificate;

const FULCIO: &str = "https://fulcio.sigstore.dev/api/v2/signingCert";
// PyPI verifies Rekor v1 entries with an integrated timestamp.
// See <https://github.com/pypi/pypi-attestations/issues/147>.
const REKOR: &str = "https://rekor.sigstore.dev/api/v1/log/entries";
const PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

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

/// Sign the serialized in-toto statement using a CI identity token for the `sigstore` audience.
pub async fn sign_statement(
    client: &Client,
    token: &str,
    statement: &str,
) -> Result<SignedStatement> {
    sign_with_services(client, FULCIO, REKOR, token, statement).await
}

async fn sign_with_services(
    client: &Client,
    fulcio: &str,
    rekor: &str,
    token: &str,
    statement: &str,
) -> Result<SignedStatement> {
    check_identity_token(token)?;
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let pem = request_certificate(client, fulcio, token, &key)
        .await
        .context("Failed to obtain a signing certificate from Fulcio")?;
    let der = CertificateDer::from_pem_slice(pem.as_bytes())?;
    let (remaining, cert) = parse_x509_certificate(&der)
        .map_err(|_| anyhow::anyhow!("Invalid Fulcio signing certificate"))?;
    ensure!(
        remaining.is_empty() && cert.public_key().raw == key.subject_public_key_info(),
        "Fulcio returned a certificate for a different key or an invalid certificate"
    );

    // DSSE signs the payload type and the exact payload bytes, each prefixed with its byte length.
    let pae = format!(
        "DSSEv1 {} {PAYLOAD_TYPE} {} {statement}",
        PAYLOAD_TYPE.len(),
        statement.len()
    );
    let signature = BASE64_STANDARD.encode(key.sign(pae.as_bytes())?);
    let payload = BASE64_STANDARD.encode(statement);
    let envelope = serde_json::to_string(&json!({
        "payloadType": PAYLOAD_TYPE,
        "payload": payload,
        "signatures": [{"sig": signature}],
    }))?;
    let entry = rekor::record(client, rekor, &envelope, statement, &signature, &pem)
        .await
        .context("Failed to record the publish attestation in Rekor")?;
    Ok(SignedStatement {
        payload,
        signature,
        certificate: BASE64_STANDARD.encode(der),
        transparency_entry: entry,
    })
}

fn check_identity_token(token: &str) -> Result<()> {
    // Check the intended audience before sending the token. Fulcio authenticates the identity.
    let (_, payload, _) = token
        .split('.')
        .collect_tuple()
        .context("Invalid Sigstore identity token")?;
    let payload = BASE64_URL_SAFE_NO_PAD
        .decode(payload)
        .context("Invalid Sigstore identity token")?;
    let claims: Value =
        serde_json::from_slice(&payload).context("Invalid Sigstore identity token")?;
    let audience = &claims["aud"];
    ensure!(
        (audience == "sigstore"
            || audience
                .as_array()
                .is_some_and(|aud| aud.iter().any(|a| a == "sigstore")))
            && claims["exp"]
                .as_i64()
                .is_some_and(|exp| exp > Timestamp::now().as_second()),
        "Expected an unexpired OIDC token with the `sigstore` audience"
    );
    Ok(())
}

async fn request_certificate(
    client: &Client,
    url: &str,
    token: &str,
    key: &KeyPair,
) -> Result<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Response {
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
    let csr = params.serialize_request(key)?.pem()?;
    let response: Response = client
        .post(url)
        .bearer_auth(token)
        .json(&json!({"certificateSigningRequest": BASE64_STANDARD.encode(csr)}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    response
        .signed_certificate_embedded_sct
        .chain
        .certificates
        .into_iter()
        .next()
        .context("Fulcio returned an empty certificate chain")
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use base64::Engine as _;
    use base64::prelude::{BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
    use rcgen::{CertificateParams, CertificateSigningRequestParams, Issuer, KeyPair};
    use reqwest::Client;
    use rustls_pki_types::{CertificateDer, pem::PemObject as _};
    use serde_json::{Value, json};
    use sha2::{Digest as _, Sha256};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    use x509_parser::{asn1_rs::BitString, parse_x509_certificate, verify::verify_signature};

    use super::sign_with_services;

    #[tokio::test]
    async fn sign_in_toto_statement() -> Result<(), Box<dyn Error>> {
        let server = MockServer::start().await;
        let issuer = Issuer::new(CertificateParams::default(), KeyPair::generate()?);
        let claims = BASE64_URL_SAFE_NO_PAD.encode(
            br#"{"iss":"https://ci.example","sub":"publisher","aud":"sigstore","exp":4102444800}"#,
        );
        let token = format!("header.{claims}.signature");
        Mock::given(method("POST"))
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
        Mock::given(method("POST"))
            .and(path("/api/v1/log/entries"))
            .respond_with(|request: &Request| {
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
                let root = Sha256::new()
                    .chain_update([0])
                    .chain_update(&body)
                    .finalize();
                let root_b64 = BASE64_STANDARD.encode(root);
                let checkpoint = format!("test-log\n1\n{root_b64}\n\n— test-log AAAAAAE=\n");
                ResponseTemplate::new(201).set_body_json(json!({"test-entry": {
                    "body": BASE64_STANDARD.encode(body), "integratedTime": 1, "logIndex": 0,
                    "logID": "00".repeat(32),
                    "verification": {
                        "signedEntryTimestamp": "AQ==",
                        "inclusionProof": {
                            "rootHash": hex::encode(root),
                            "logIndex": 0, "treeSize": 1, "hashes": [],
                            "checkpoint": checkpoint,
                        }
                    }
                }}))
            })
            .expect(1)
            .mount(&server)
            .await;
        let statement = serde_json::to_string(&json!({
            "_type": "https://in-toto.io/Statement/v1",
            "subject": [],
            "predicateType": "https://example.com/test",
        }))?;
        let signed = sign_with_services(
            &Client::new(),
            &format!("{}/api/v2/signingCert", server.uri()),
            &format!("{}/api/v1/log/entries", server.uri()),
            &token,
            &statement,
        )
        .await?;

        let requests = server.received_requests().await.expect("Requests recorded");
        let entry: Value = requests[1].body_json()?;
        let proposed = &entry["spec"]["proposedContent"];
        let envelope: Value =
            serde_json::from_str(proposed["envelope"].as_str().expect("DSSE envelope"))?;
        assert_eq!(signed.signature, envelope["signatures"][0]["sig"]);
        assert_eq!(signed.payload, envelope["payload"]);
        assert_eq!(
            BASE64_STANDARD.decode(&signed.payload)?,
            statement.as_bytes()
        );
        let pem = String::from_utf8(
            BASE64_STANDARD.decode(
                proposed["verifiers"][0]
                    .as_str()
                    .expect("Rekor certificate"),
            )?,
        )?;
        let der = CertificateDer::from_pem_slice(pem.as_bytes())?;
        assert_eq!(signed.certificate, BASE64_STANDARD.encode(&der));
        let receipt = &signed.transparency_entry;
        assert_eq!(receipt["logIndex"], "0");
        assert_eq!(receipt["inclusionProof"]["treeSize"], "1");
        let (_, cert) = parse_x509_certificate(&der)?;
        let signature = BASE64_STANDARD.decode(
            envelope["signatures"][0]["sig"]
                .as_str()
                .expect("Signature"),
        )?;
        let payload = BASE64_STANDARD.decode(envelope["payload"].as_str().expect("Payload"))?;
        let mut pae =
            format!("DSSEv1 28 application/vnd.in-toto+json {} ", payload.len()).into_bytes();
        pae.extend_from_slice(&payload);
        verify_signature(
            cert.public_key(),
            &cert.signature_algorithm,
            &BitString::new(0, &signature),
            &pae,
        )?;
        Ok(())
    }
}
