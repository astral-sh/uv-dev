use std::assert_matches;
use std::str::FromStr;

use anyhow::Result;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, ErrorKind, RegistryClientBuilder};
use uv_distribution_filename::SourceDistExtension;
use uv_distribution_types::{File, FileLocation, IndexUrl, RegistrySourceDist};
use uv_extract::hash::Hasher;
use uv_pypi_types::{HashAlgorithm, HashDigest, HashDigests};
use uv_redacted::DisplaySafeUrl;

const METADATA: &str =
    "Metadata-Version: 2.4\nName: example\nVersion: 1.0\nRequires-Dist: dependency>=1\n\n";

fn metadata_hash(metadata: &str) -> HashDigests {
    let mut hasher = Hasher::from(HashAlgorithm::Sha256);
    hasher.update(metadata.as_bytes());
    HashDigest::from(hasher).into()
}

fn distribution(server: &MockServer, metadata: &str) -> Result<RegistrySourceDist> {
    let url = DisplaySafeUrl::parse(&format!("{}/example-1.0.tar.gz", server.uri()))?;
    Ok(RegistrySourceDist {
        name: "example".parse()?,
        version: "1.0".parse()?,
        file: Box::new(File {
            dist_info_metadata: Some(metadata_hash(metadata)),
            filename: "example-1.0.tar.gz".into(),
            hashes: HashDigests::empty(),
            requires_python: None,
            size: None,
            upload_time_utc_ms: None,
            url: FileLocation::AbsoluteUrl(url.into()),
            yanked: None,
        }),
        ext: SourceDistExtension::TarGz,
        index: IndexUrl::from_str(&format!("{}/simple", server.uri()))?,
        wheels: Vec::new(),
        size_is_authoritative: false,
    })
}

#[tokio::test]
async fn source_sidecars_are_cached_by_advertised_hash() -> Result<()> {
    let server = MockServer::start().await;
    let updated = METADATA.replace("dependency>=1", "dependency>=2");
    Mock::given(method("GET"))
        .and(path("/example-1.0.tar.gz.metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=3600")
                .set_body_string(METADATA),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/example-1.0.tar.gz.metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=3600")
                .set_body_string(&updated),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client =
        RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp()?.init().await?)
            .build()?;
    let first = distribution(&server, METADATA)?;
    for _ in 0..2 {
        let metadata = client
            .source_metadata(&first)
            .await?
            .expect("static source metadata");
        assert_eq!(metadata.requires_dist[0].to_string(), "dependency>=1");
    }
    let second = distribution(&server, &updated)?;
    let metadata = client
        .source_metadata(&second)
        .await?
        .expect("updated source metadata");
    assert_eq!(metadata.requires_dist[0].to_string(), "dependency>=2");
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn source_sidecar_hash_mismatch_is_an_error() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/example-1.0.tar.gz.metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_string("different metadata"))
        .expect(1)
        .mount(&server)
        .await;
    let client =
        RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp()?.init().await?)
            .build()?;
    let error = client
        .source_metadata(&distribution(&server, METADATA)?)
        .await
        .expect_err("sidecar hash must be checked");
    assert_matches!(error.kind(), ErrorKind::MetadataHashMismatch { .. });
    server.verify().await;
    Ok(())
}

#[tokio::test]
async fn unavailable_or_non_static_sidecars_fall_back() -> Result<()> {
    for metadata in [
        METADATA.replace("Metadata-Version: 2.4", "Metadata-Version: 2.1"),
        METADATA.replace("Requires-Dist:", "Dynamic: Requires-Dist\nRequires-Dist:"),
        METADATA.replace("Name: example", "Name: different"),
        METADATA.replace("Version: 1.0", "Version: 2.0"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/example-1.0.tar.gz.metadata"))
            .respond_with(ResponseTemplate::new(200).set_body_string(&metadata))
            .expect(1)
            .mount(&server)
            .await;
        let client =
            RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp()?.init().await?)
                .build()?;
        assert!(
            client
                .source_metadata(&distribution(&server, &metadata)?)
                .await?
                .is_none()
        );
        server.verify().await;
    }

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/example-1.0.tar.gz.metadata"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let client =
        RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp()?.init().await?)
            .build()?;
    assert!(
        client
            .source_metadata(&distribution(&server, METADATA)?)
            .await?
            .is_none()
    );
    server.verify().await;
    Ok(())
}
