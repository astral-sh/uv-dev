use std::future::Future;
use std::io::{self, Cursor};
use std::pin::Pin;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::header::{
    ACCEPT_ENCODING, ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, ETAG, HeaderValue,
    IF_NONE_MATCH, RANGE,
};
use reqwest::{Method, Request as HttpRequest, Url};
use serde_json::json;
use tokio::time::{sleep, timeout};
use tracing_test::traced_test;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use uv_cache::{Cache, CacheBucket, CacheEntry, Freshness, Refresh, WheelCache};
use uv_client::{
    AuthIntegration, BaseClientBuilder, CacheControl, Connectivity, MetadataRangeRequest,
    RegistryClient, RegistryClientBuilder,
};
use uv_distribution_filename::WheelFilename;
use uv_distribution_types::{
    BuiltDist, File, FileLocation, Index, IndexCapabilities, IndexLocations, IndexUrl,
    RegistryBuiltDist, RegistryBuiltWheel,
};
use uv_extract::hash::Hasher;
use uv_fs::write_atomic;
use uv_git::GitResolver;
use uv_metadata::read_archive_metadata;
use uv_pypi_types::{HashAlgorithm, HashDigest, HashDigests, ResolutionMetadata};
use uv_redacted::DisplaySafeUrl;

use super::cached_client::cached_payload;
use super::remote_metadata::{wheel, wheel_range_response};

const WHEEL_FILENAME: &str = "ok-1.0.0-py3-none-any.whl";
const WHEEL_PATH: &str = "/ok-1.0.0-py3-none-any.whl";
const FRESH: &str = "public, max-age=31536000";
const ETAG_VALUE: &str = "\"v1\"";
const TEST_TIMEOUT: Duration = Duration::from_secs(10);
const CACHE_HIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
enum MetadataRoute {
    Pep658,
    Range,
    Stream,
}

impl MetadataRoute {
    const ALL: [Self; 3] = [Self::Pep658, Self::Range, Self::Stream];

    fn method(self) -> &'static str {
        match self {
            Self::Pep658 | Self::Stream => "GET",
            Self::Range => "HEAD",
        }
    }
}

struct MetadataFixture {
    route: MetadataRoute,
    dist: BuiltDist,
    entry: CacheEntry,
    lock_entry: CacheEntry,
    index: IndexUrl,
    policy_url: DisplaySafeUrl,
    metadata: Vec<u8>,
    archive: Vec<u8>,
}

impl MetadataFixture {
    fn new(server: &MockServer, cache: &Cache, route: MetadataRoute) -> Result<Self> {
        let filename = WheelFilename::from_str(WHEEL_FILENAME)?;
        let archive = wheel()?;
        let metadata = read_archive_metadata(&filename, Cursor::new(&archive))?;
        let file_url = DisplaySafeUrl::parse(&format!("{}{WHEEL_PATH}", server.uri()))?;
        let mut policy_url = file_url.clone();
        let dist_info_metadata = match route {
            MetadataRoute::Pep658 => {
                let metadata_path = format!("{}.metadata", policy_url.path());
                policy_url.set_path(&metadata_path);
                let mut hasher = Hasher::from(HashAlgorithm::Sha256);
                hasher.update(&metadata);
                Some(std::iter::once(HashDigest::from(hasher)).collect())
            }
            MetadataRoute::Range | MetadataRoute::Stream => None,
        };
        let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
        let base_url = format!("{}/", server.uri()).into();
        let file = File {
            dist_info_metadata,
            filename: WHEEL_FILENAME.into(),
            hashes: HashDigests::empty(),
            requires_python: None,
            size: Some(u64::try_from(archive.len())?),
            upload_time_utc_ms: None,
            url: FileLocation::new(file_url.to_string().into(), &base_url),
            yanked: None,
        };
        let dist = BuiltDist::Registry(RegistryBuiltDist {
            wheels: vec![RegistryBuiltWheel {
                filename: filename.clone(),
                file: Box::new(file),
                index: index.clone(),
                size_is_authoritative: true,
            }],
            best_wheel_index: 0,
            sdist: None,
        });
        let entry = cache.entry(
            CacheBucket::Wheels,
            WheelCache::Index(&index).wheel_dir(filename.name.as_ref()),
            format!("{}.msgpack", filename.cache_key()),
        );
        #[cfg(windows)]
        let lock_key = filename.stem();
        #[cfg(not(windows))]
        let lock_key = filename.cache_key();
        let lock_entry = entry.with_file(format!("{lock_key}.lock"));
        Ok(Self {
            route,
            dist,
            entry,
            lock_entry,
            index,
            policy_url,
            metadata,
            archive,
        })
    }

    async fn mount(
        &self,
        server: &MockServer,
        cache_control: &'static str,
        initial_requests: u64,
        revalidation_requests: u64,
    ) {
        let response = match self.route {
            MetadataRoute::Pep658 => ResponseTemplate::new(200)
                .set_body_raw(self.metadata.clone(), "application/octet-stream"),
            MetadataRoute::Range => ResponseTemplate::new(200)
                .insert_header(ACCEPT_RANGES, "bytes")
                .insert_header(CONTENT_LENGTH, self.archive.len().to_string()),
            MetadataRoute::Stream => {
                ResponseTemplate::new(200).set_body_raw(self.archive.clone(), "application/zip")
            }
        }
        .insert_header(CACHE_CONTROL, cache_control)
        .insert_header(ETAG, ETAG_VALUE);
        Mock::given(method(self.route.method()))
            .and(path(self.policy_url.path()))
            .and(|request: &Request| !request.headers.contains_key(IF_NONE_MATCH))
            .respond_with(response)
            .expect(initial_requests)
            .mount(server)
            .await;
        Mock::given(method(self.route.method()))
            .and(path(self.policy_url.path()))
            .and(header(IF_NONE_MATCH.as_str(), ETAG_VALUE))
            .respond_with(
                ResponseTemplate::new(304)
                    .insert_header(CACHE_CONTROL, cache_control)
                    .insert_header(ETAG, ETAG_VALUE),
            )
            .expect(revalidation_requests)
            .mount(server)
            .await;
        match self.route {
            MetadataRoute::Pep658 | MetadataRoute::Stream => {}
            MetadataRoute::Range => {
                let archive = self.archive.clone();
                Mock::given(method("GET"))
                    .and(path(WHEEL_PATH))
                    .and(header_exists(RANGE.as_str()))
                    .respond_with(move |request: &Request| wheel_range_response(request, &archive))
                    .expect(1)
                    .mount(server)
                    .await;
            }
        }
    }

    fn client(
        &self,
        cache: Cache,
        connectivity: Connectivity,
        cache_control: Option<&str>,
    ) -> Result<RegistryClient> {
        let mut index = Index::from_index_url(self.index.clone());
        if let Some(cache_control) = cache_control {
            index.cache_control = Some(serde_json::from_value(json!({ "files": cache_control }))?);
        }
        let metadata_range_request = match self.route {
            MetadataRoute::Pep658 | MetadataRoute::Range => MetadataRangeRequest::Require,
            MetadataRoute::Stream => MetadataRangeRequest::Fallback,
        };
        Ok(RegistryClientBuilder::new(
            BaseClientBuilder::default()
                .custom_client(reqwest::Client::builder().no_proxy().build()?)
                .auth_integration(AuthIntegration::NoAuthMiddleware)
                .connectivity(connectivity)
                .metadata_range_request(metadata_range_request)
                .retries(0),
            cache,
        )
        .index_locations(IndexLocations::new(vec![index], Vec::new(), false))
        .build()?)
    }

    fn request(&self) -> HttpRequest {
        let url = Url::from(self.policy_url.clone());
        let method = match self.route {
            MetadataRoute::Pep658 | MetadataRoute::Stream => Method::GET,
            MetadataRoute::Range => Method::HEAD,
        };
        let mut request = HttpRequest::new(method, url);
        match self.route {
            MetadataRoute::Pep658 => {}
            MetadataRoute::Range | MetadataRoute::Stream => {
                request
                    .headers_mut()
                    .insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
            }
        }
        request
    }

    async fn read(&self, client: &RegistryClient) -> Result<ResolutionMetadata> {
        let capabilities = IndexCapabilities::default();
        match self.route {
            MetadataRoute::Pep658 | MetadataRoute::Range => {}
            MetadataRoute::Stream => capabilities.set_no_range_requests(self.index.clone()),
        }
        self.read_with_capabilities(client, &capabilities).await
    }

    async fn read_with_capabilities(
        &self,
        client: &RegistryClient,
        capabilities: &IndexCapabilities,
    ) -> Result<ResolutionMetadata> {
        Ok(client
            .wheel_metadata(&self.dist, &GitResolver::default(), capabilities, None)
            .await?)
    }

    fn waiting_message(&self) -> String {
        format!(
            "Waiting to acquire exclusive lock for `{}`",
            self.lock_entry.path().display()
        )
    }
}

fn assert_metadata(metadata: &ResolutionMetadata) {
    assert_eq!(metadata.name.to_string(), "ok");
    assert_eq!(metadata.version.to_string(), "1.0.0");
}

/// Wait for the actual lock-contention event, keeping the metadata future alive and polled.
async fn wait_until_lock_wait<F>(mut waiter: Pin<&mut F>, waiting: impl Fn() -> bool) -> Result<()>
where
    F: Future<Output = Result<ResolutionMetadata>>,
{
    timeout(TEST_TIMEOUT, async {
        loop {
            tokio::select! {
                result = waiter.as_mut() => {
                    return Err(anyhow!("Metadata request completed before lock release: {result:?}"));
                }
                () = sleep(Duration::from_millis(5)) => {
                    if waiting() {
                        return Ok(());
                    }
                }
            }
        }
    })
    .await
    .context("metadata request did not reach the publication lock")?
}

/// Choose a refresh cutoff after the seed entry, including on coarse timestamp filesystems.
async fn refreshed_cache(cache: &Cache, entry: &CacheEntry) -> Result<Cache> {
    timeout(TEST_TIMEOUT, async {
        loop {
            let refreshed = cache
                .clone()
                .with_refresh(Refresh::from_args(Some(true), Vec::new()));
            if refreshed.freshness(entry, None, None)? == Freshness::Stale {
                return Ok(refreshed);
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .context("cache refresh cutoff did not pass the seed entry")?
}

#[tokio::test]
async fn fresh_wheel_metadata_does_not_wait_for_publication() -> Result<()> {
    for route in MetadataRoute::ALL {
        let server = MockServer::start().await;
        let cache = Cache::temp()?.init().await?;
        let fixture = MetadataFixture::new(&server, &cache, route)?;
        fixture.mount(&server, FRESH, 1, 0).await;
        let online = fixture.client(cache.clone(), Connectivity::Online, None)?;
        assert_metadata(&fixture.read(&online).await?);

        let _lock = fixture.lock_entry.lock().await?;
        for connectivity in [Connectivity::Online, Connectivity::Offline] {
            let client = fixture.client(cache.clone(), connectivity, None)?;
            assert_metadata(&timeout(CACHE_HIT_TIMEOUT, fixture.read(&client)).await??);
        }
        server.verify().await;
    }
    Ok(())
}

#[tokio::test]
async fn streamed_wheel_metadata_is_reused_by_range_requests() -> Result<()> {
    let server = MockServer::start().await;
    let cache = Cache::temp()?.init().await?;
    let fixture = MetadataFixture::new(&server, &cache, MetadataRoute::Stream)?;
    fixture.mount(&server, FRESH, 1, 0).await;
    let online = fixture.client(cache.clone(), Connectivity::Online, None)?;
    assert_metadata(&fixture.read(&online).await?);

    let _lock = fixture.lock_entry.lock().await?;
    for connectivity in [Connectivity::Online, Connectivity::Offline] {
        let client = fixture.client(cache.clone(), connectivity, None)?;
        // A new process initially assumes the index supports range requests.
        let capabilities = IndexCapabilities::default();
        assert_metadata(
            &timeout(
                CACHE_HIT_TIMEOUT,
                fixture.read_with_capabilities(&client, &capabilities),
            )
            .await??,
        );
    }
    server.verify().await;
    Ok(())
}

#[tokio::test]
#[traced_test]
async fn stale_wheel_metadata_is_only_reused_offline() -> Result<()> {
    for route in MetadataRoute::ALL {
        let server = MockServer::start().await;
        let cache = Cache::temp()?.init().await?;
        let fixture = MetadataFixture::new(&server, &cache, route)?;
        fixture.mount(&server, "public, max-age=0", 1, 1).await;
        let online = fixture.client(cache.clone(), Connectivity::Online, None)?;
        assert_metadata(&fixture.read(&online).await?);

        let lock = fixture.lock_entry.lock().await?;
        let offline = fixture.client(cache, Connectivity::Offline, None)?;
        assert_metadata(&timeout(CACHE_HIT_TIMEOUT, fixture.read(&offline)).await??);

        let waiter = fixture.read(&online);
        tokio::pin!(waiter);
        let waiting_message = fixture.waiting_message();
        wait_until_lock_wait(waiter.as_mut(), || logs_contain(&waiting_message)).await?;
        drop(lock);
        assert_metadata(&timeout(TEST_TIMEOUT, waiter).await??);
        server.verify().await;
    }
    Ok(())
}

#[tokio::test]
#[traced_test]
async fn refresh_waiter_rechecks_published_wheel_metadata() -> Result<()> {
    for route in MetadataRoute::ALL {
        let server = MockServer::start().await;
        let cache = Cache::temp()?.init().await?;
        let fixture = MetadataFixture::new(&server, &cache, route)?;
        fixture.mount(&server, FRESH, 1, 1).await;
        let online = fixture.client(cache.clone(), Connectivity::Online, None)?;
        assert_metadata(&fixture.read(&online).await?);
        let refreshed = refreshed_cache(&cache, &fixture.entry).await?;
        let client = fixture.client(refreshed.clone(), Connectivity::Online, None)?;

        let lock = fixture.lock_entry.lock().await?;
        let waiter = fixture.read(&client);
        tokio::pin!(waiter);
        let waiting_message = fixture.waiting_message();
        wait_until_lock_wait(waiter.as_mut(), || logs_contain(&waiting_message)).await?;
        assert_eq!(
            refreshed.freshness(&fixture.entry, None, None)?,
            Freshness::Stale
        );

        // This publisher holds the same wheel lock and refreshes the existing entry with a 304.
        let metadata = online
            .cached_client()
            .get_serde_with_retry(
                fixture.request(),
                &fixture.entry,
                CacheControl::MustRevalidate,
                async |response, _retry_state| {
                    Err::<ResolutionMetadata, _>(io::Error::other(format!(
                        "Expected revalidation without a body, got {}",
                        response.status(),
                    )))
                },
            )
            .await
            .map_err(|error| anyhow!("{error:?}"))?;
        assert_metadata(&metadata);
        assert_eq!(
            refreshed.freshness(&fixture.entry, None, None)?,
            Freshness::Fresh
        );
        drop(lock);

        assert_metadata(&timeout(TEST_TIMEOUT, waiter).await??);
        server.verify().await;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum CacheCorruption {
    Policy,
    Payload,
}

#[tokio::test]
#[traced_test]
async fn malformed_wheel_metadata_waiter_keeps_replacement() -> Result<()> {
    for route in MetadataRoute::ALL {
        for corruption in [CacheCorruption::Policy, CacheCorruption::Payload] {
            for connectivity in [Connectivity::Online, Connectivity::Offline] {
                let server = MockServer::start().await;
                let cache = Cache::temp()?.init().await?;
                let fixture = MetadataFixture::new(&server, &cache, route)?;
                let initial_requests = match corruption {
                    CacheCorruption::Policy => 1,
                    CacheCorruption::Payload => 2,
                };
                fixture.mount(&server, FRESH, initial_requests, 0).await;
                let online = fixture.client(cache.clone(), Connectivity::Online, None)?;
                assert_metadata(&fixture.read(&online).await?);
                let valid = fs_err::read(fixture.entry.path())?;

                let lock = fixture.lock_entry.lock().await?;
                match corruption {
                    CacheCorruption::Policy => {
                        // A complete cache envelope cannot fit in seven bytes.
                        write_atomic(fixture.entry.path(), [0_u8; 7]).await?;
                    }
                    CacheCorruption::Payload => {
                        online
                            .cached_client()
                            .skip_cache_with_retry(
                                fixture.request(),
                                &fixture.entry,
                                CacheControl::None,
                                async |response, _retry_state| {
                                    response.bytes().await.map(|_| 42_u64)
                                },
                            )
                            .await
                            .map_err(|error| anyhow!("{error:?}"))?;
                        assert_eq!(cached_payload::<u64>(&fixture.entry)?, 42);
                    }
                }
                assert!(cached_payload::<ResolutionMetadata>(&fixture.entry).is_err());
                let invalid = fs_err::read(fixture.entry.path())?;

                let client = fixture.client(cache, connectivity, None)?;
                let waiter = fixture.read(&client);
                tokio::pin!(waiter);
                let waiting_message = fixture.waiting_message();
                wait_until_lock_wait(waiter.as_mut(), || logs_contain(&waiting_message)).await?;
                assert_eq!(fs_err::read(fixture.entry.path())?, invalid);

                // A permitted writer publishes a replacement before the waiting reader resumes.
                write_atomic(fixture.entry.path(), &valid).await?;
                drop(lock);
                assert_metadata(&timeout(TEST_TIMEOUT, waiter).await??);
                assert_eq!(fs_err::read(fixture.entry.path())?, valid);
                server.verify().await;
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn wheel_metadata_hit_uses_index_cache_control() -> Result<()> {
    for route in MetadataRoute::ALL {
        let server = MockServer::start().await;
        let cache = Cache::temp()?.init().await?;
        let fixture = MetadataFixture::new(&server, &cache, route)?;
        fixture.mount(&server, "no-store", 1, 0).await;
        let client = fixture.client(cache, Connectivity::Online, Some(FRESH))?;
        assert_metadata(&fixture.read(&client).await?);
        assert_metadata(&cached_payload::<ResolutionMetadata>(&fixture.entry)?);

        let _lock = fixture.lock_entry.lock().await?;
        assert_metadata(&timeout(CACHE_HIT_TIMEOUT, fixture.read(&client)).await??);
        server.verify().await;
    }
    Ok(())
}
