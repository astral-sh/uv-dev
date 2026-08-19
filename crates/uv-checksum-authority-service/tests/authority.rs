use std::process::Command;

use anyhow::{Result, anyhow};
use assert_cmd::assert::OutputAssertExt;
use assert_fs::prelude::*;
use ring::signature::Ed25519KeyPair;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use url::Url;
use uv_checksum_authority::{
    ArtifactId, AuthorityPublicKey, ChecksumAuthority, ChecksumRecord, Error, Sha256Digest,
    SignedRecord,
};
use uv_checksum_authority_service::{AuthorityService, Catalog};
use uv_test::uv_snapshot;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Responses remain untrusted until both the signature and identity have been checked.
#[tokio::test]
async fn rejects_untrusted_authority_responses() -> Result<()> {
    let server = MockServer::start().await;
    let signing_key = key(7)?;
    let record = record()?;
    let authority = ChecksumAuthority::new(
        Url::parse(&server.uri())?,
        AuthorityPublicKey::from_signing_key(&signing_key),
    )?;

    Mock::given(method("GET"))
        .and(path("/v1/checksum"))
        .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(16 * 1024 + 1)))
        .mount(&server)
        .await;
    insta::assert_snapshot!(authority.lookup(record.artifact()).await.expect_err("oversized response"), @"Checksum authority response exceeds the size limit");

    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v1/checksum"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/redirect"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/redirect"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    insta::assert_snapshot!(authority.lookup(record.artifact()).await.expect_err("redirect"), @"Checksum authority returned HTTP 302 Found");

    server.reset().await;
    let other = ChecksumRecord::new(
        ArtifactId::from_canonical(
            "https://another.example/simple",
            record.artifact().filename(),
        )?,
        record.sha256(),
        record.size(),
    );
    Mock::given(method("GET"))
        .and(path("/v1/checksum"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(SignedRecord::sign(&other, &signing_key)?),
        )
        .mount(&server)
        .await;
    insta::assert_snapshot!(authority.lookup(record.artifact()).await.expect_err("wrong identity"), @"Checksum authority returned a record for a different artifact");
    Ok(())
}

fn key(seed: u8) -> Result<Ed25519KeyPair> {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).map_err(|_| anyhow!("invalid test key"))
}

fn record() -> Result<ChecksumRecord> {
    Ok(ChecksumRecord::new(
        ArtifactId::new(
            &Url::parse("https://pypi.org/simple/")?,
            "example-1.0-py3-none-any.whl",
        )?,
        Sha256Digest::from_bytes(Sha256::digest(b"trusted archive").into()),
        15,
    ))
}

#[tokio::test]
async fn in_memory_authority() -> Result<()> {
    let record = record()?;
    let signing_key = key(7)?;
    let service = AuthorityService::new(Catalog::from_records([record.clone()])?, &signing_key)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = Url::parse(&format!("http://{}", listener.local_addr()?))?;
    let authority = ChecksumAuthority::new(endpoint.clone(), service.public_key())?;
    let (shutdown, stopped) = oneshot::channel();
    let server = tokio::spawn(service.serve(listener, async {
        let _ = stopped.await;
    }));

    assert_eq!(authority.lookup(record.artifact()).await?.record(), &record);
    let unknown = ArtifactId::from_canonical(record.artifact().source(), "missing-1.0.tar.gz")?;
    assert!(matches!(
        authority.lookup(&unknown).await,
        Err(Error::UnknownArtifact(_))
    ));
    let wrong_key =
        ChecksumAuthority::new(endpoint, AuthorityPublicKey::from_signing_key(&key(8)?))?;
    assert!(matches!(
        wrong_key.lookup(record.artifact()).await,
        Err(Error::InvalidSignature)
    ));

    let temporary = tempfile::tempdir()?;
    let original = http::Response::new("trusted archive");
    let verified = authority
        .verify_response(original.into(), record.artifact(), temporary.path())
        .await?;
    assert_eq!(verified.bytes().await?.as_ref(), b"trusted archive");
    let replacement = http::Response::new("untrusted bytes");
    assert!(matches!(
        authority
            .verify_response(replacement.into(), record.artifact(), temporary.path())
            .await,
        Err(Error::Mismatch { .. })
    ));

    shutdown
        .send(())
        .map_err(|()| anyhow!("server exited early"))?;
    server.await??;
    Ok(())
}

#[test]
fn immutable_catalog_and_signed_identity() -> Result<()> {
    let record = record()?;
    let mut catalog = Catalog::from_records([record.clone(), record.clone()])?;
    let conflict = ChecksumRecord::new(record.artifact().clone(), [0; 32].into(), record.size());
    insta::assert_snapshot!(catalog.insert(conflict).expect_err("conflict must fail"), @"Conflicting checksum for `example-1.0-py3-none-any.whl`; existing records cannot be replaced");
    assert_eq!(catalog.records().count(), 1);

    let signing_key = key(7)?;
    let signed = SignedRecord::sign(&record, &signing_key)?;
    let public_key = AuthorityPublicKey::from_signing_key(&signing_key);
    let other = ArtifactId::from_canonical(
        "https://another.example/simple",
        record.artifact().filename(),
    )?;
    assert!(matches!(
        signed.verify(&other, &public_key),
        Err(Error::WrongArtifact)
    ));
    let mut envelope = serde_json::to_value(signed)?;
    envelope["payload"] = serde_json::Value::String("dGFtcGVyZWQ=".to_owned());
    let modified: SignedRecord = serde_json::from_value(envelope)?;
    assert!(matches!(
        modified.verify(record.artifact(), &public_key),
        Err(Error::InvalidSignature)
    ));
    Ok(())
}

#[test]
fn identities_do_not_leak_credentials() -> Result<()> {
    let identity = ArtifactId::new(
        &Url::parse("https://user:secret@example.com/simple/#fragment")?,
        "example.whl",
    )?;
    assert_eq!(identity.source(), "https://example.com/simple");
    assert!(
        ArtifactId::new(
            &Url::parse("https://example.com/simple?token=secret")?,
            "example.whl"
        )
        .is_err()
    );
    assert!(ArtifactId::new(&Url::parse("https://example.com/simple")?, "../example.whl").is_err());
    assert!(
        ChecksumAuthority::new(Url::parse("http://example.com")?, "00".repeat(32).parse()?)
            .is_err()
    );
    Ok(())
}

#[test]
fn catalog_cli_is_append_only() -> Result<()> {
    let directory = assert_fs::TempDir::new()?;
    let key = directory.child("authority.key");
    let catalog = directory.child("catalog.json");
    let archive = directory.child("example.whl");
    let binary = env!("CARGO_BIN_EXE_uv-checksum-authority-service");

    uv_snapshot!([(r"[a-f0-9]{64}", "[PUBLIC_KEY]")], Command::new(binary)
        .args(["keygen", "--signing-key"])
        .arg(key.path()), @"
    exit_code: 0 (success)
    ----- stdout -----
    [PUBLIC_KEY]
    ");
    let original_key = fs_err::read(key.path())?;
    Command::new(binary)
        .args(["keygen", "--signing-key"])
        .arg(key.path())
        .assert()
        .failure();
    assert_eq!(fs_err::read(key.path())?, original_key);

    archive.write_str("trusted archive")?;
    let add = || {
        let mut command = Command::new(binary);
        command
            .args(["add", "--catalog"])
            .arg(catalog.path())
            .args(["--source", "https://pypi.org/simple"])
            .arg(archive.path());
        command
    };
    uv_snapshot!(add(), @"
    exit_code: 0 (success)
    ");
    let original = fs_err::read(catalog.path())?;
    uv_snapshot!(add(), @"
    exit_code: 0 (success)
    ");
    assert_eq!(fs_err::read(catalog.path())?, original);
    archive.write_str("replacement")?;
    uv_snapshot!(add(), @"
    exit_code: 1 (failure)
    ----- stderr -----
    Error: Conflicting checksum for `example.whl`; existing records cannot be replaced
    ");
    assert_eq!(fs_err::read(catalog.path())?, original);
    Ok(())
}

/// Invalid wire values must fail during deserialization, not at their next use.
#[test]
fn parse_record() -> Result<()> {
    let record = record()?;
    let value = serde_json::to_value(&record)?;
    assert_eq!(
        serde_json::from_value::<ChecksumRecord>(value.clone())?,
        record
    );

    let mut invalid = value.clone();
    invalid["artifact"]["source"] = "https://user:secret@pypi.org/simple".into();
    insta::assert_snapshot!(serde_json::from_value::<ChecksumRecord>(invalid).expect_err("noncanonical source"), @"Invalid checksum authority artifact identity");

    let mut invalid = value.clone();
    invalid["artifact"]["filename"] = "../example.whl".into();
    insta::assert_snapshot!(serde_json::from_value::<ChecksumRecord>(invalid).expect_err("invalid filename"), @"Invalid checksum authority artifact identity");

    for digest in ["ab".to_owned(), "AB".repeat(32), "gg".repeat(32)] {
        let mut invalid = value.clone();
        invalid["sha256"] = digest.into();
        insta::allow_duplicates! {
            insta::assert_snapshot!(serde_json::from_value::<ChecksumRecord>(invalid).expect_err("invalid digest"), @"Checksum authority records require a lowercase SHA-256 digest");
        }
    }
    let mut invalid = value;
    invalid
        .as_object_mut()
        .expect("record object")
        .remove("size");
    insta::assert_snapshot!(serde_json::from_value::<ChecksumRecord>(invalid).expect_err("missing size"), @"missing field `size`");
    Ok(())
}

/// A signed size bounds the archive stream even when Content-Length is absent or dishonest.
#[tokio::test]
async fn archive_size_and_status() -> Result<()> {
    let record = record()?;
    let service = AuthorityService::new(Catalog::from_records([record.clone()])?, &key(7)?)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let authority = ChecksumAuthority::new(
        Url::parse(&format!("http://{}", listener.local_addr()?))?,
        service.public_key(),
    )?;
    let (shutdown, stopped) = oneshot::channel();
    let server = tokio::spawn(service.serve(listener, async {
        let _ = stopped.await;
    }));
    let temporary = assert_fs::TempDir::new()?;

    for body in ["short", "an archive longer than admitted"] {
        let response = http::Response::new(body);
        let error = authority
            .verify_response(response.into(), record.artifact(), temporary.path())
            .await
            .expect_err("wrong size");
        insta::allow_duplicates! {
            insta::assert_snapshot!(error, @"Checksum authority size mismatch for `example-1.0-py3-none-any.whl`: expected 15 bytes");
        }
    }
    let response = http::Response::builder()
        .status(206)
        .body("trusted archive")?;
    insta::assert_snapshot!(authority.verify_response(response.into(), record.artifact(), temporary.path()).await.expect_err("partial response"), @"Expected a complete archive response, received HTTP 206 Partial Content");

    shutdown
        .send(())
        .map_err(|()| anyhow!("server exited early"))?;
    server.await??;
    Ok(())
}

#[tokio::test]
async fn rejects_malformed_queries() -> Result<()> {
    let service = AuthorityService::new(Catalog::default(), &key(7)?)?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let (shutdown, stopped) = oneshot::channel();
    let server = tokio::spawn(service.serve(listener, async {
        let _ = stopped.await;
    }));
    let client = reqwest::Client::new();
    for query in [
        "source=https://pypi.org/simple&filename=a.whl&filename=b.whl",
        "source=https://pypi.org/simple&filename=../a.whl",
        "source=https://pypi.org/simple/&filename=a.whl",
        "source=https://pypi.org/simple&filename=a.whl&extra=1",
        "source=https://pypi.org/simple",
    ] {
        let response = client
            .get(format!("{endpoint}/v1/checksum?{query}"))
            .send()
            .await?;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
    let response = client
        .get(format!(
            "{endpoint}/v1/checksum?source={}",
            "a".repeat(8500)
        ))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::URI_TOO_LONG);
    let response = client.get(format!("{endpoint}/health")).send().await?;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    insta::assert_snapshot!(response.text().await?, @"ok");
    shutdown
        .send(())
        .map_err(|()| anyhow!("server exited early"))?;
    server.await??;
    Ok(())
}
