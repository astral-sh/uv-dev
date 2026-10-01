use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use uv_cache::{Cache, Refresh};
use uv_client::{BaseClientBuilder, RegistryClient, RegistryClientBuilder};
use uv_distribution_filename::{SourceDistExtension, WheelFilename};
use uv_distribution_types::{
    BuiltDist, File, FileLocation, IndexCapabilities, IndexUrl, RegistryBuiltDist,
    RegistryBuiltWheel, RegistrySourceDist,
};
use uv_extract::hash::Hasher;
use uv_git::GitResolver;
use uv_pypi_types::{HashAlgorithm, HashDigest, HashDigests, ResolutionMetadata};
use uv_redacted::DisplaySafeUrl;

const METADATA: &str =
    "Metadata-Version: 2.4\nName: example\nVersion: 1.0\nRequires-Dist: dependency>=1\n\n";

fn metadata_hash(metadata: &str, algorithm: Option<HashAlgorithm>) -> HashDigests {
    let Some(algorithm) = algorithm else {
        return HashDigests::empty();
    };
    let mut hasher = Hasher::from(algorithm);
    hasher.update(metadata.as_bytes());
    HashDigest::from(hasher).into()
}

fn filename(wheel: bool) -> &'static str {
    if wheel {
        "example-1.0-py3-none-any.whl"
    } else {
        "example-1.0.tar.gz"
    }
}

async fn read_metadata(
    client: &RegistryClient,
    server: &MockServer,
    wheel: bool,
    hashes: HashDigests,
) -> Result<ResolutionMetadata> {
    let filename = filename(wheel);
    let url = DisplaySafeUrl::parse(&format!("{}/{filename}", server.uri()))?;
    let file = Box::new(File {
        dist_info_metadata: Some(hashes),
        filename: filename.into(),
        hashes: HashDigests::empty(),
        requires_python: None,
        size: None,
        upload_time_utc_ms: None,
        url: FileLocation::AbsoluteUrl(url.into()),
        yanked: None,
    });
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    if wheel {
        Ok(client
            .wheel_metadata(
                &BuiltDist::Registry(RegistryBuiltDist {
                    wheels: vec![RegistryBuiltWheel {
                        filename: WheelFilename::from_str(filename)?,
                        file,
                        index,
                        size_is_authoritative: false,
                    }],
                    best_wheel_index: 0,
                    sdist: None,
                }),
                &GitResolver::default(),
                &IndexCapabilities::default(),
                None,
            )
            .await?)
    } else {
        Ok(client
            .source_metadata(&RegistrySourceDist {
                name: "example".parse()?,
                version: "1.0".parse()?,
                file,
                ext: SourceDistExtension::TarGz,
                index,
                wheels: Vec::new(),
                size_is_authoritative: false,
            })
            .await?
            .expect("static source metadata"))
    }
}

#[tokio::test]
async fn strong_hashes_identify_cached_sidecars() -> Result<()> {
    for wheel in [false, true] {
        for lifetime in ["public, max-age=0", "public, max-age=3600"] {
            let server = MockServer::start().await;
            let generation = Arc::new(AtomicUsize::new(0));
            let requests = Arc::new(AtomicUsize::new(0));
            let updated = METADATA.replace("dependency>=1", "dependency>=2");
            let response_generation = generation.clone();
            let response_requests = requests.clone();
            let response_updated = updated.clone();
            Mock::given(method("GET"))
                .and(path(format!("/{}.metadata", filename(wheel))))
                .respond_with(move |_: &Request| {
                    response_requests.fetch_add(1, Ordering::SeqCst);
                    ResponseTemplate::new(200)
                        .insert_header("Cache-Control", lifetime)
                        .set_body_string(if response_generation.load(Ordering::SeqCst) == 0 {
                            METADATA
                        } else {
                            &response_updated
                        })
                })
                .mount(&server)
                .await;
            let cache = Cache::temp()?.init().await?;
            let client =
                RegistryClientBuilder::new(BaseClientBuilder::default(), cache.clone()).build()?;
            for _ in 0..2 {
                let metadata = read_metadata(
                    &client,
                    &server,
                    wheel,
                    metadata_hash(METADATA, Some(HashAlgorithm::Sha256)),
                )
                .await?;
                assert_eq!(metadata.requires_dist[0].to_string(), "dependency>=1");
            }
            assert_eq!(requests.load(Ordering::SeqCst), 1);
            generation.store(1, Ordering::SeqCst);
            let metadata = read_metadata(
                &client,
                &server,
                wheel,
                metadata_hash(&updated, Some(HashAlgorithm::Sha256)),
            )
            .await?;
            assert_eq!(metadata.requires_dist[0].to_string(), "dependency>=2");
            assert_eq!(requests.load(Ordering::SeqCst), 2);

            let client = RegistryClientBuilder::new(
                BaseClientBuilder::default(),
                cache.with_refresh(Refresh::from_args(Some(true), Vec::new())),
            )
            .build()?;
            let metadata = read_metadata(
                &client,
                &server,
                wheel,
                metadata_hash(&updated, Some(HashAlgorithm::Sha256)),
            )
            .await?;
            assert_eq!(metadata.requires_dist[0].to_string(), "dependency>=2");
            assert_eq!(requests.load(Ordering::SeqCst), 3);
        }
    }
    Ok(())
}

#[tokio::test]
async fn weak_or_missing_hashes_revalidate_stale_sidecars() -> Result<()> {
    for wheel in [false, true] {
        for algorithm in [None, Some(HashAlgorithm::Md5)] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!("/{}.metadata", filename(wheel))))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("Cache-Control", "public, max-age=0")
                        .set_body_string(METADATA),
                )
                .expect(2)
                .mount(&server)
                .await;
            let client = RegistryClientBuilder::new(
                BaseClientBuilder::default(),
                Cache::temp()?.init().await?,
            )
            .build()?;
            for _ in 0..2 {
                let metadata =
                    read_metadata(&client, &server, wheel, metadata_hash(METADATA, algorithm))
                        .await?;
                assert_eq!(metadata.requires_dist[0].to_string(), "dependency>=1");
            }
            server.verify().await;
        }
    }
    Ok(())
}
