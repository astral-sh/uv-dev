use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use async_zip::base::write::ZipFileWriter;
use async_zip::{Compression, ZipEntryBuilder};
use reqwest::header::{
    ACCEPT_RANGES, AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, HeaderName, LOCATION, RANGE,
};
use wiremock::matchers::{basic_auth, header_exists, header_regex, method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, MetadataRangeRequest, RegistryClientBuilder};
use uv_distribution_filename::WheelFilename;
use uv_distribution_types::{BuiltDist, DirectUrlBuiltDist, IndexCapabilities};
use uv_git::GitResolver;
use uv_pep508::VerbatimUrl;
use uv_redacted::DisplaySafeUrl;

#[tokio::test]
async fn remote_metadata_with_and_without_cache() -> Result<()> {
    let server = MockServer::start().await;
    let wheel = fs_err::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/links/ok-1.0.0-py3-none-any.whl"),
    )?;
    Mock::given(method("GET"))
        .and(path("/ok-1.0.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(wheel, "application/octet-stream"))
        .mount(&server)
        .await;

    let cache = Cache::temp()?.init().await?;
    let client = RegistryClientBuilder::new(BaseClientBuilder::default(), cache).build()?;

    // The first run is without cache (the tempdir is empty), the second has the cache from the
    // first run.
    for _ in 0..2 {
        let url = format!("{}/ok-1.0.0-py3-none-any.whl", server.uri());
        let filename = WheelFilename::from_str("ok-1.0.0-py3-none-any.whl")?;
        let dist = BuiltDist::DirectUrl(DirectUrlBuiltDist {
            filename,
            location: Box::new(DisplaySafeUrl::parse(&url)?),
            url: VerbatimUrl::from_str(&url)?,
            size: None,
        });
        let resolver = GitResolver::default();
        let capabilities = IndexCapabilities::default();
        let metadata = client
            .wheel_metadata(&dist, &resolver, &capabilities, None)
            .await?;
        assert_eq!(metadata.version.to_string(), "1.0.0");
    }

    Ok(())
}

#[tokio::test]
async fn remote_metadata_requires_range_requests() -> Result<()> {
    let server = MockServer::start().await;
    let wheel = fs_err::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/links/ok-1.0.0-py3-none-any.whl"),
    )?;
    Mock::given(method("GET"))
        .and(path("/ok-1.0.0-py3-none-any.whl"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(wheel, "application/octet-stream"))
        .mount(&server)
        .await;

    let cache = Cache::temp()?.init().await?;
    let client = RegistryClientBuilder::new(
        BaseClientBuilder::default().metadata_range_request(MetadataRangeRequest::Require),
        cache,
    )
    .build()?;

    let url = format!("{}/ok-1.0.0-py3-none-any.whl", server.uri());
    let filename = WheelFilename::from_str("ok-1.0.0-py3-none-any.whl")?;
    let dist = BuiltDist::DirectUrl(DirectUrlBuiltDist {
        filename,
        location: Box::new(DisplaySafeUrl::parse(&url)?),
        url: VerbatimUrl::from_str(&url)?,
        size: None,
    });
    let resolver = GitResolver::default();
    let capabilities = IndexCapabilities::default();
    let error = client
        .wheel_metadata(&dist, &resolver, &capabilities, None)
        .await
        .expect_err("range requests should be required");

    insta::assert_snapshot!(
        error.to_string().replace(&server.uri(), "[HOST]"),
        @"Wheel metadata range requests are required, but not supported for: `[HOST]/ok-1.0.0-py3-none-any.whl`"
    );

    Ok(())
}

/// Covers same-origin redirect semantics and credential propagation.
#[tokio::test]
async fn remote_metadata_redirect_same_origin() -> Result<()> {
    let server = MockServer::start().await;
    let wheel = wheel()?;
    let wheel_len = wheel.len();

    // The initial metadata probe should authenticate to the source and receive a redirect.
    Mock::given(method("HEAD"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .respond_with(
            ResponseTemplate::new(303)
                .insert_header(LOCATION, format!("{}/head-wheel", server.uri())),
        )
        .expect(1)
        .named("HEAD request to the redirecting wheel URL")
        .mount(&server)
        .await;
    // The range reader should retry the source with an authenticated range request.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_exists(RANGE.as_str()))
        .respond_with(
            ResponseTemplate::new(303)
                .insert_header(LOCATION, format!("{}/head-wheel", server.uri())),
        )
        .expect(1)
        .named("ranged GET request to the redirecting wheel URL")
        .mount(&server)
        .await;
    // The target supports ranges, so metadata should be read without starting a streaming
    // download. A fallback here would hide a failure to follow the range request's redirect.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_missing(RANGE))
        .respond_with(
            ResponseTemplate::new(303)
                .insert_header(LOCATION, format!("{}/head-wheel", server.uri())),
        )
        .expect(0)
        .named("streaming fallback GET request to the redirecting wheel URL")
        .mount(&server)
        .await;
    // The redirected `HEAD` request should retain credentials on the same origin.
    Mock::given(method("HEAD"))
        .and(path("/head-wheel"))
        .and(basic_auth("source-user", "source-password"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(ACCEPT_RANGES, "bytes")
                .insert_header(CONTENT_LENGTH, wheel_len.to_string()),
        )
        .expect(1)
        .named("HEAD request to the same-origin redirect target")
        .mount(&server)
        .await;
    let ranged_wheel = wheel.clone();
    // Range requests must follow redirects just like the initial `HEAD` probe; otherwise,
    // redirected wheels cannot benefit from fetching only the bytes needed for metadata.
    Mock::given(method("GET"))
        .and(path("/head-wheel"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_exists(RANGE.as_str()))
        .respond_with(move |request: &Request| wheel_range_response(request, &ranged_wheel))
        .expect(1)
        .named("ranged GET request to the same-origin redirect target")
        .mount(&server)
        .await;
    // The range response already provides the metadata. Streaming the wheel as well would
    // transfer unnecessary data and could mask a failure in the range-reading path.
    Mock::given(method("GET"))
        .and(path("/head-wheel"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(200).set_body_raw(wheel, "application/octet-stream"))
        .expect(0)
        .named("streaming GET request to the same-origin redirect target")
        .mount(&server)
        .await;

    assert_wheel_metadata_readable(&server).await?;

    Ok(())
}

/// Models registries that redirect wheels to another artifact origin, such as Azure Artifacts
/// redirecting to `vsblob.vsassets.io` or Gemfury and pypicloud redirecting to Amazon S3. The source
/// `Authorization` header must not be forwarded to the artifact host.
#[tokio::test]
async fn remote_metadata_redirect_cross_origin() -> Result<()> {
    let source_server = MockServer::start().await;
    let target_server = MockServer::start().await;
    let wheel = wheel()?;
    let wheel_len = wheel.len();
    let target = format!("{}/head-wheel", target_server.uri());

    // The initial metadata probe should authenticate to the source and receive a redirect.
    Mock::given(method("HEAD"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, target.clone()))
        .expect(1)
        .named("HEAD request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // The range reader should retry the source with an authenticated range request.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_exists(RANGE.as_str()))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, target.clone()))
        .expect(1)
        .named("ranged GET request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // Moving the wheel to another origin should not force a streaming download when that
    // origin supports ranges. Metadata should still be read from a partial response.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, target))
        .expect(0)
        .named("streaming fallback GET request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // The redirected `HEAD` request should omit the source credentials on the new origin.
    Mock::given(method("HEAD"))
        .and(path("/head-wheel"))
        .and(header_missing(AUTHORIZATION))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(ACCEPT_RANGES, "bytes")
                .insert_header(CONTENT_LENGTH, wheel_len.to_string()),
        )
        .expect(1)
        .named("unauthenticated HEAD request to the cross-origin redirect target")
        .mount(&target_server)
        .await;
    let ranged_wheel = wheel.clone();
    // The range request must reach the artifact host, but that host may be controlled by a
    // different party. Strip the source credentials so following the redirect cannot leak them.
    Mock::given(method("GET"))
        .and(path("/head-wheel"))
        .and(header_missing(AUTHORIZATION))
        .and(header_exists(RANGE.as_str()))
        .respond_with(move |request: &Request| wheel_range_response(request, &ranged_wheel))
        .expect(1)
        .named("unauthenticated ranged GET request to the cross-origin redirect target")
        .mount(&target_server)
        .await;
    // The artifact host can serve the required range without source credentials. A streaming
    // fallback would hide a failure to complete that range request.
    Mock::given(method("GET"))
        .and(path("/head-wheel"))
        .and(header_missing(AUTHORIZATION))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(200).set_body_raw(wheel, "application/octet-stream"))
        .expect(0)
        .named("unauthenticated streaming GET request to the cross-origin redirect target")
        .mount(&target_server)
        .await;

    assert_wheel_metadata_readable(&source_server).await?;

    Ok(())
}

/// Models registries that issue method-specific signed redirects, such as Gemfury and pypicloud
/// backed by Amazon S3 (astral-sh/uv#2025 and astral-sh/uv#3255) and the public Microsoft package
/// feed backed by Azure Artifacts (astral-sh/uv#21347).
#[tokio::test]
async fn remote_metadata_redirect_method_specific_target() -> Result<()> {
    let source_server = MockServer::start().await;
    let target_server = MockServer::start().await;
    // Separate the metadata from the central directory so reading it requires multiple ranges.
    let mut writer = ZipFileWriter::new(Vec::new());
    writer
        .write_entry_whole(
            ZipEntryBuilder::new("ok-1.0.0.dist-info/METADATA".into(), Compression::Stored),
            b"Metadata-Version: 2.1\nName: ok\nVersion: 1.0.0\n",
        )
        .await?;
    writer
        .write_entry_whole(
            ZipEntryBuilder::new("padding".into(), Compression::Stored),
            &[0; 32_768],
        )
        .await?;
    let wheel = writer.close().await?;
    let wheel_len = wheel.len();
    let head_target = authenticated_url(
        &target_server.uri(),
        "/head-wheel",
        "head-user",
        "head-password",
    )?;
    let get_target = authenticated_url(
        &target_server.uri(),
        "/get-wheel",
        "get-user",
        "get-password",
    )?;

    // The initial authenticated probe should receive the signed `HEAD` target.
    Mock::given(method("HEAD"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, head_target))
        .expect(1)
        .named("HEAD request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // Both ranges must start at the source to obtain a URL valid for `GET`. Reusing the `HEAD`
    // destination would fail on servers that include the HTTP method in the URL signature.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_exists(RANGE.as_str()))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, get_target.clone()))
        .expect(2)
        .named("ranged GET request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // Both required ranges are available through the `GET` redirect. Falling back to streaming
    // would let this test pass even if ranged requests still could not follow that redirect.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, get_target))
        .expect(0)
        .named("streaming fallback GET request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // The redirected probe should use the credentials embedded in the signed `HEAD` target.
    Mock::given(method("HEAD"))
        .and(path("/head-wheel"))
        .and(basic_auth("head-user", "head-password"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(ACCEPT_RANGES, "bytes")
                .insert_header(CONTENT_LENGTH, wheel_len.to_string()),
        )
        .expect(1)
        .named("HEAD request to the method-specific HEAD redirect target")
        .mount(&target_server)
        .await;
    let ranged_wheel = wheel.clone();
    // The central directory and metadata are in separate ranges. Both must use the `GET`
    // destination and its credentials so neither depends on a URL signed only for `HEAD`.
    Mock::given(method("GET"))
        .and(path("/get-wheel"))
        .and(basic_auth("get-user", "get-password"))
        .and(header_exists(RANGE.as_str()))
        .respond_with(move |request: &Request| wheel_range_response(request, &ranged_wheel))
        .expect(2)
        .named("ranged GET request to the method-specific GET redirect target")
        .mount(&target_server)
        .await;
    // The two partial responses should suffice to read the metadata. A full `GET` would bypass
    // the range-reading behavior that this test is intended to protect.
    Mock::given(method("GET"))
        .and(path("/get-wheel"))
        .and(basic_auth("get-user", "get-password"))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(200).set_body_raw(wheel, "application/octet-stream"))
        .expect(0)
        .named("streaming GET request to the method-specific GET redirect target")
        .mount(&target_server)
        .await;
    // A `GET` request should not reuse the signed `HEAD` target.
    Mock::given(method("GET"))
        .and(path("/head-wheel"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .named("GET request must not reuse the HEAD redirect target")
        .mount(&target_server)
        .await;
    // A `HEAD` request should not reuse the signed `GET` target.
    Mock::given(method("HEAD"))
        .and(path("/get-wheel"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .named("HEAD request must not reuse the GET redirect target")
        .mount(&target_server)
        .await;

    assert_wheel_metadata_readable(&source_server).await?;

    Ok(())
}

/// Some servers support bounded ranges but reject suffix ranges. Wheel metadata should be read with
/// a bounded range request, without attempting a suffix range or streaming fallback.
#[tokio::test]
async fn remote_metadata_bounded_ranges() -> Result<()> {
    let server = MockServer::start().await;
    let wheel = wheel()?;
    // The initial `HEAD` response should advertise bounded range support and the artifact length.
    Mock::given(method("HEAD"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(ACCEPT_RANGES, "bytes")
                .insert_header(CONTENT_LENGTH, wheel.len().to_string()),
        )
        .expect(1)
        .mount(&server)
        .await;
    // The metadata should be read with a bounded range request.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_regex(RANGE.as_str(), "^bytes=[0-9]+-[0-9]+$"))
        .respond_with(move |request: &Request| wheel_range_response(request, &wheel))
        .expect(1)
        .named("bounded range request")
        .mount(&server)
        .await;
    // A suffix range request should not be sent when bounded ranges are supported.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(header_regex(RANGE.as_str(), "^bytes=-"))
        .respond_with(ResponseTemplate::new(416))
        .expect(0)
        .named("unsupported suffix range request")
        .mount(&server)
        .await;
    // A streaming fallback should not be needed when the bounded range request succeeds.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .named("unnecessary streaming fallback")
        .mount(&server)
        .await;

    assert_wheel_metadata_readable(&server).await
}

/// Documents a compatibility limitation exposed by following range redirects.
/// This synthetic target advertises ranges and permits streaming, but rejects ranged `GET` with
/// `403`. Previously, the unfollowed redirect triggered streaming before uv ever reached that
/// rejection. The range reader's `403` does not currently trigger a streaming fallback; restoring
/// that fallback is proposed separately in astral-sh/uv-dev#917.
#[tokio::test]
async fn remote_metadata_redirect_range_forbidden() -> Result<()> {
    let source_server = MockServer::start().await;
    let target_server = MockServer::start().await;
    let wheel = wheel()?;
    let target = format!("{}/wheel", target_server.uri());
    // The initial metadata probe should authenticate to the source and receive a redirect.
    Mock::given(method("HEAD"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, target.clone()))
        .expect(1)
        .mount(&source_server)
        .await;
    // The range reader should retry the source with an authenticated range request.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_exists(RANGE.as_str()))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, target.clone()))
        .expect(1)
        .named("ranged GET request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // The current fallback policy does not classify the range reader's `403` as unsupported
    // ranges, so it returns the error without retrying the source with a full `GET`.
    Mock::given(method("GET"))
        .and(path("/artifact"))
        .and(basic_auth("source-user", "source-password"))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(303).insert_header(LOCATION, target))
        .expect(0)
        .named("streaming fallback GET request to the redirecting wheel URL")
        .mount(&source_server)
        .await;
    // The redirected `HEAD` request should omit the source credentials, and its response should
    // advertise range support.
    Mock::given(method("HEAD"))
        .and(path("/wheel"))
        .and(header_missing(AUTHORIZATION))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(ACCEPT_RANGES, "bytes")
                .insert_header(CONTENT_LENGTH, wheel.len().to_string()),
        )
        .expect(1)
        .mount(&target_server)
        .await;
    // We must reach this response to exercise the target's rejection. Previously, the
    // unfollowed redirect caused a fallback before this response was ever returned.
    Mock::given(method("GET"))
        .and(path("/wheel"))
        .and(header_missing(AUTHORIZATION))
        .and(header_exists(RANGE.as_str()))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .named("forbidden range request to the redirect target")
        .mount(&target_server)
        .await;
    // This endpoint would allow metadata extraction by streaming, but the unhandled range
    // error currently prevents that fallback. Keep it here to make the compatibility gap clear.
    Mock::given(method("GET"))
        .and(path("/wheel"))
        .and(header_missing(AUTHORIZATION))
        .and(header_missing(RANGE))
        .respond_with(ResponseTemplate::new(200).set_body_raw(wheel, "application/octet-stream"))
        .expect(0)
        .named("streaming GET request to the redirect target")
        .mount(&target_server)
        .await;

    let error = assert_wheel_metadata_readable(&source_server)
        .await
        .expect_err("the redirect target should reject the range request");
    insta::assert_snapshot!(
        error.root_cause().to_string().replace(&target_server.uri(), "[TARGET]"),
        @"HTTP status client error (403 Forbidden) for url ([TARGET]/wheel)"
    );
    Ok(())
}

#[derive(Debug)]
struct HeaderMissing(HeaderName);

impl Match for HeaderMissing {
    fn matches(&self, request: &Request) -> bool {
        !request.headers.contains_key(&self.0)
    }
}

/// Matches requests that omit a header, complementing Wiremock's `header_exists` matcher.
fn header_missing(header: HeaderName) -> HeaderMissing {
    HeaderMissing(header)
}

/// Loads the wheel fixture served by each redirect target.
fn wheel() -> Result<Vec<u8>> {
    Ok(fs_err::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/links/ok-1.0.0-py3-none-any.whl"),
    )?)
}

/// Reads wheel metadata through the authenticated source URL shared by each redirect scenario.
async fn assert_wheel_metadata_readable(source_server: &MockServer) -> Result<()> {
    let cache = Cache::temp()?.init().await?;
    let client = RegistryClientBuilder::new(BaseClientBuilder::default(), cache).build()?;
    let url = authenticated_url(
        &source_server.uri(),
        "/artifact",
        "source-user",
        "source-password",
    )?;
    let dist = BuiltDist::DirectUrl(DirectUrlBuiltDist {
        filename: WheelFilename::from_str("ok-1.0.0-py3-none-any.whl")?,
        location: Box::new(DisplaySafeUrl::parse(&url)?),
        url: VerbatimUrl::from_str(&url)?,
        size: None,
    });
    let metadata = client
        .wheel_metadata(
            &dist,
            &GitResolver::default(),
            &IndexCapabilities::default(),
            None,
        )
        .await?;
    assert_eq!(metadata.version.to_string(), "1.0.0");
    Ok(())
}

/// Adds Basic authentication credentials to a Wiremock server URL.
fn authenticated_url(base: &str, path: &str, username: &str, password: &str) -> Result<String> {
    Ok(format!(
        "http://{username}:{password}@{}{path}",
        base.strip_prefix("http://")
            .context("mock server URL should use HTTP")?
    ))
}

/// Serves a byte range from the wheel fixture, as an artifact host would.
fn wheel_range_response(request: &Request, wheel: &[u8]) -> ResponseTemplate {
    let Some((start, end)) = request
        .headers
        .get(RANGE)
        .and_then(|range| range.to_str().ok())
        .and_then(|range| parse_byte_range(range, wheel.len()))
    else {
        return ResponseTemplate::new(416)
            .insert_header(ACCEPT_RANGES, "bytes")
            .insert_header(CONTENT_RANGE, format!("bytes */{}", wheel.len()));
    };
    ResponseTemplate::new(206)
        .insert_header(ACCEPT_RANGES, "bytes")
        .insert_header(
            CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", wheel.len()),
        )
        .set_body_raw(wheel[start..=end].to_vec(), "application/octet-stream")
}

/// Parses the single byte-range forms emitted by the range reader.
fn parse_byte_range(range: &str, length: usize) -> Option<(usize, usize)> {
    let range = range.strip_prefix("bytes=")?;
    let (start, end) = range.split_once('-')?;

    if start.is_empty() {
        let suffix = end.parse::<usize>().ok()?;
        return (suffix > 0 && length > 0).then(|| (length.saturating_sub(suffix), length - 1));
    }

    let start = start.parse::<usize>().ok()?;
    if start >= length {
        return None;
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<usize>().ok()?.min(length - 1)
    };
    (start <= end).then_some((start, end))
}
