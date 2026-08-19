use std::process::Command;

use anyhow::{Result, anyhow, ensure};
use ring::signature::Ed25519KeyPair;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use url::Url;
use uv_checksum_authority::{
    ArtifactId, ChecksumAuthority, ChecksumRecord, Error, SignedRecord, public_key_hex,
};
use uv_checksum_authority_service::{AuthorityService, Catalog};

fn key(seed: u8) -> Result<Ed25519KeyPair> {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).map_err(|_| anyhow!("invalid test key"))
}

fn record() -> Result<ChecksumRecord> {
    Ok(ChecksumRecord {
        artifact: ArtifactId::new(
            &Url::parse("https://pypi.org/simple/")?,
            "example-1.0-py3-none-any.whl",
        )?,
        sha256: hex::encode(Sha256::digest(b"trusted archive")),
    })
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

    assert_eq!(authority.lookup(&record.artifact).await?, record);
    let mut unknown = record.artifact.clone();
    unknown.filename = "missing-1.0.tar.gz".to_owned();
    assert!(matches!(
        authority.lookup(&unknown).await,
        Err(Error::UnknownArtifact(_))
    ));
    let wrong_key = ChecksumAuthority::new(endpoint, &public_key_hex(&key(8)?))?;
    assert!(matches!(
        wrong_key.lookup(&record.artifact).await,
        Err(Error::InvalidSignature)
    ));

    let temporary = tempfile::tempdir()?;
    let original = http::Response::new("trusted archive");
    let verified = authority
        .verify_response(original.into(), &record.artifact, temporary.path())
        .await?;
    assert_eq!(verified.bytes().await?.as_ref(), b"trusted archive");
    let replacement = http::Response::new("replacement archive");
    assert!(matches!(
        authority
            .verify_response(replacement.into(), &record.artifact, temporary.path())
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
    let mut conflict = record.clone();
    conflict.sha256 = "0".repeat(64);
    insta::assert_snapshot!(catalog.insert(conflict).expect_err("conflict must fail"), @"Conflicting checksum for `example-1.0-py3-none-any.whl`; existing records cannot be replaced");
    assert_eq!(catalog.records().count(), 1);

    let signing_key = key(7)?;
    let signed = SignedRecord::sign(&record, &signing_key)?;
    let mut public_key = [0; 32];
    hex::decode_to_slice(public_key_hex(&signing_key), &mut public_key)?;
    let mut other = record.artifact.clone();
    other.source = "https://another.example/simple".to_owned();
    assert!(matches!(
        signed.verify(&other, &public_key),
        Err(Error::WrongArtifact)
    ));
    let mut modified = signed;
    modified.payload.push('A');
    assert!(matches!(
        modified.verify(&record.artifact, &public_key),
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
    assert_eq!(identity.source, "https://example.com/simple");
    assert!(
        ArtifactId::new(
            &Url::parse("https://example.com/simple?token=secret")?,
            "example.whl"
        )
        .is_err()
    );
    assert!(ArtifactId::new(&Url::parse("https://example.com/simple")?, "../example.whl").is_err());
    assert!(ChecksumAuthority::new(Url::parse("http://example.com")?, &"00".repeat(32)).is_err());
    Ok(())
}

#[test]
fn catalog_cli_is_append_only() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let key = directory.path().join("authority.key");
    let catalog = directory.path().join("catalog.json");
    let archive = directory.path().join("example.whl");
    let binary = env!("CARGO_BIN_EXE_uv-checksum-authority-service");
    let output = Command::new(binary)
        .args(["keygen", "--signing-key"])
        .arg(&key)
        .output()?;
    ensure!(
        output.status.success(),
        "keygen failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let public_key = String::from_utf8(output.stdout)?;
    ensure!(public_key.trim().len() == 64);
    ensure!(
        !Command::new(binary)
            .args(["keygen", "--signing-key"])
            .arg(&key)
            .output()?
            .status
            .success()
    );

    fs_err::write(&archive, b"trusted archive")?;
    let add = || {
        Command::new(binary)
            .args(["add", "--catalog"])
            .arg(&catalog)
            .args(["--source", "https://pypi.org/simple"])
            .arg(&archive)
            .output()
    };
    ensure!(add()?.status.success());
    let original = fs_err::read(&catalog)?;
    ensure!(add()?.status.success());
    assert_eq!(fs_err::read(&catalog)?, original);
    fs_err::write(&archive, b"replacement")?;
    let rejected = add()?;
    ensure!(!rejected.status.success());
    insta::assert_snapshot!(String::from_utf8(rejected.stderr)?, @"Error: Conflicting checksum for `example.whl`; existing records cannot be replaced");
    assert_eq!(fs_err::read(&catalog)?, original);
    Ok(())
}
