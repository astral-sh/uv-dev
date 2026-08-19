use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use tokio::sync::Semaphore;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_cache::Cache;
use uv_client::{
    BaseClientBuilder, Connectivity, MetadataFormat, OwnedArchive, RegistryClient,
    RegistryClientBuilder,
};
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{
    BuiltDist, DistInfoMetadata, Index, IndexCapabilities, IndexLocations, IndexUrl,
    RegistryBuiltDist, RegistryBuiltWheel,
};
use uv_git::GitResolver;
use uv_normalize::PackageName;
use uv_pypi_types::ResolutionMetadata;
use uv_redacted::DisplaySafeUrl;

const WHEEL: &str = "ok-1.0.0-py3-none-any.whl";
const METADATA: &str = "Metadata-Version: 2.3\nName: ok\nVersion: 1.0.0\n\n";

fn client(cache: Cache, index: &IndexUrl) -> Result<RegistryClient> {
    Ok(
        RegistryClientBuilder::new(BaseClientBuilder::default(), cache)
            .index_locations(IndexLocations::new(
                vec![Index::from_index_url(index.clone())],
                vec![],
                false,
            ))
            .build()?,
    )
}

async fn distribution(client: &RegistryClient) -> Result<BuiltDist> {
    let name = PackageName::from_str("ok")?;
    let capabilities = IndexCapabilities::default();
    let semaphore = Semaphore::new(1);
    let responses = client
        .simple_detail(&name, None, &capabilities, &semaphore)
        .await?;
    let (index, format) = responses.into_iter().next().context("missing index")?;
    let MetadataFormat::Simple(archive) = format else {
        anyhow::bail!("expected Simple API metadata");
    };
    let (filename, file) = OwnedArchive::deserialize(&archive)
        .into_iter()
        .flat_map(|datum| datum.files.all(&name))
        .next()
        .context("missing wheel")?;
    let DistFilename::WheelFilename(filename) = filename else {
        anyhow::bail!("expected wheel");
    };
    Ok(BuiltDist::Registry(RegistryBuiltDist {
        wheels: vec![RegistryBuiltWheel {
            filename,
            file: Box::new(file),
            index: index.clone(),
            size_is_authoritative: false,
        }],
        best_wheel_index: 0,
        sdist: None,
    }))
}

async fn metadata(client: &RegistryClient, dist: &BuiltDist) -> Result<ResolutionMetadata> {
    Ok(client
        .wheel_metadata(
            dist,
            &GitResolver::default(),
            &IndexCapabilities::default(),
            None,
        )
        .await?)
}

fn simple(url: &str, advertisement: Option<bool>, json: bool) -> ResponseTemplate {
    let response = if json {
        let mut file = serde_json::json!({"filename": WHEEL, "url": url, "hashes": {}});
        if let Some(available) = advertisement {
            file["core-metadata"] = available.into();
        }
        ResponseTemplate::new(200).set_body_raw(
            serde_json::json!({"files": [file]}).to_string(),
            "application/vnd.pypi.simple.v1+json",
        )
    } else {
        let advertisement = advertisement
            .map(|available| format!(" data-core-metadata=\"{available}\""))
            .unwrap_or_default();
        ResponseTemplate::new(200).set_body_raw(
            format!("<a href=\"{url}\"{advertisement}>{WHEEL}</a>"),
            "text/html",
        )
    };
    response.insert_header("Cache-Control", "max-age=3600")
}

async fn mount_wheel(server: &MockServer) -> Result<()> {
    let wheel = fs_err::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test/links")
            .join(WHEEL),
    )?;
    Mock::given(method("HEAD"))
        .and(path(format!("/{WHEEL}")))
        .respond_with(ResponseTemplate::new(405))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "max-age=3600")
                .set_body_raw(wheel, "application/octet-stream"),
        )
        .mount(server)
        .await;
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_discovery() -> Result<()> {
    for json in [false, true] {
        for (product, advertisement, expected) in [
            (
                Some("Artifactory/7.0.0"),
                None,
                DistInfoMetadata::Unadvertised,
            ),
            (
                Some("Artifactory/7.0.0"),
                Some(false),
                DistInfoMetadata::Unavailable,
            ),
            (
                Some("Artifactory/7.0.0"),
                Some(true),
                DistInfoMetadata::Available,
            ),
            (Some("Other/7.0.0"), None, DistInfoMetadata::Unavailable),
            (None, None, DistInfoMetadata::Unavailable),
            (None, Some(true), DistInfoMetadata::Available),
        ] {
            let server = MockServer::start().await;
            let mut response = simple(&format!("/{WHEEL}"), advertisement, json);
            if let Some(product) = product {
                response = response.insert_header("X-JFrog-Version", product);
            }
            Mock::given(method("GET"))
                .and(path("/simple/ok/"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
            let cache = Cache::temp()?.init().await?;
            // A second client must recover the per-file capability from the cached Simple response.
            for _ in 0..2 {
                let client = client(cache.clone(), &index)?;
                let BuiltDist::Registry(dist) = distribution(&client).await? else {
                    anyhow::bail!("expected registry distribution");
                };
                assert_eq!(dist.best_wheel().file.dist_info_metadata, expected);
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_requires_same_origin() -> Result<()> {
    let origin = MockServer::start().await;
    let other = MockServer::start().await;
    for (index, target) in [
        (
            format!("{}/simple", origin.uri()),
            format!("{}/{WHEEL}", other.uri()),
        ),
        (
            format!("{}/redirect", origin.uri()),
            format!("{}/{WHEEL}", origin.uri()),
        ),
    ] {
        origin.reset().await;
        other.reset().await;
        Mock::given(method("GET"))
            .and(path("/simple/ok/"))
            .respond_with(
                simple(&target, None, true).insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
            )
            .mount(&origin)
            .await;
        Mock::given(method("GET"))
            .and(path("/redirect/ok/"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/simple/ok/", other.uri())),
            )
            .mount(&origin)
            .await;
        Mock::given(method("GET"))
            .and(path("/simple/ok/"))
            .respond_with(
                simple(&target, None, true).insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
            )
            .mount(&other)
            .await;
        let client = client(Cache::temp()?.init().await?, &IndexUrl::from_str(&index)?)?;
        let BuiltDist::Registry(dist) = distribution(&client).await? else {
            anyhow::bail!("expected registry distribution");
        };
        assert_eq!(
            dist.best_wheel().file.dist_info_metadata,
            DistInfoMetadata::Unavailable
        );
    }
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_matches_advertised_behavior() -> Result<()> {
    // Different dependencies are accepted just as they are for an advertised digestless sidecar.
    let sidecar = METADATA.replace("\n\n", "\nRequires-Dist: other==2\n\n");
    for advertisement in [None, Some(true)] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/ok/"))
            .respond_with(
                simple(
                    &format!("/{WHEEL}?download=1#sha256=abcd"),
                    advertisement,
                    false,
                )
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{WHEEL}.metadata")))
            .and(query_param("download", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Cache-Control", "max-age=3600")
                    .set_body_string(&sidecar),
            )
            .expect(1)
            .mount(&server)
            .await;
        let cache = Cache::temp()?.init().await?;
        let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
        for _ in 0..2 {
            let client = client(cache.clone(), &index)?;
            let dist = distribution(&client).await?;
            let metadata = metadata(&client, &dist).await?;
            assert_eq!(metadata.name.as_ref(), "ok");
            assert_eq!(metadata.version.to_string(), "1.0.0");
            assert_eq!(metadata.requires_dist.len(), 1);
        }
        assert_eq!(
            server.received_requests().await.context("requests")?.len(),
            2
        );
    }
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_falls_back() -> Result<()> {
    for response in [
        ResponseTemplate::new(404),
        ResponseTemplate::new(405),
        ResponseTemplate::new(200).set_body_string("not metadata"),
        ResponseTemplate::new(200).set_body_string(METADATA.replace("Name: ok", "Name: other")),
        ResponseTemplate::new(200)
            .set_body_string(METADATA.replace("Version: 1.0.0", "Version: 2.0.0")),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/ok/"))
            .respond_with(
                simple(&format!("/{WHEEL}"), None, true)
                    .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{WHEEL}.metadata")))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        mount_wheel(&server).await?;
        let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
        let client = client(Cache::temp()?.init().await?, &index)?;
        let dist = distribution(&client).await?;
        for _ in 0..2 {
            let metadata = metadata(&client, &dist).await?;
            assert_eq!(metadata.name.as_ref(), "ok");
            assert_eq!(metadata.version.to_string(), "1.0.0");
            assert!(metadata.requires_dist.is_empty());
        }
    }
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_auth_errors_are_not_optional() -> Result<()> {
    for status in [401, 403] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/ok/"))
            .respond_with(
                simple(&format!("/{WHEEL}"), None, true)
                    .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{WHEEL}.metadata")))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
        let client = client(Cache::temp()?.init().await?, &index)?;
        let dist = distribution(&client).await?;
        assert!(metadata(&client, &dist).await.is_err());
        let requests = server.received_requests().await.context("requests")?;
        assert!(
            requests
                .iter()
                .all(|request| request.url.path() != format!("/{WHEEL}"))
        );
    }
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_revalidates() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), None, true)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}.metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Cache-Control", "max-age=0")
                .insert_header("ETag", "\"metadata\"")
                .set_body_string(METADATA),
        )
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let client = client(Cache::temp()?.init().await?, &index)?;
    let dist = distribution(&client).await?;
    metadata(&client, &dist).await?;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}.metadata")))
        .and(header("if-none-match", "\"metadata\""))
        .respond_with(ResponseTemplate::new(304).insert_header("ETag", "\"metadata\""))
        .expect(1)
        .with_priority(1)
        .mount(&server)
        .await;
    metadata(&client, &dist).await?;
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_redirect_does_not_forward_credentials() -> Result<()> {
    let origin = MockServer::start().await;
    let target = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), None, true)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
        )
        .mount(&origin)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}.metadata")))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/{WHEEL}.metadata", target.uri())),
        )
        .mount(&origin)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}.metadata")))
        .respond_with(ResponseTemplate::new(200).set_body_string(METADATA))
        .expect(1)
        .mount(&target)
        .await;
    let mut url = DisplaySafeUrl::parse(&format!("{}/simple", origin.uri()))?;
    url.set_username("user")
        .map_err(|()| anyhow::anyhow!("username"))?;
    url.set_password(Some("password"))
        .map_err(|()| anyhow::anyhow!("password"))?;
    let index = IndexUrl::from_str(url.as_str())?;
    let client = client(Cache::temp()?.init().await?, &index)?;
    let dist = distribution(&client).await?;
    metadata(&client, &dist).await?;
    let requests = origin
        .received_requests()
        .await
        .context("origin requests")?;
    let sidecar = requests
        .iter()
        .find(|request| request.url.path() == format!("/{WHEEL}.metadata"))
        .context("origin sidecar request")?;
    assert_eq!(
        sidecar
            .headers
            .get("authorization")
            .context("origin authorization")?,
        "Basic dXNlcjpwYXNzd29yZA=="
    );
    let requests = target.received_requests().await.context("requests")?;
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("authorization"));
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_offline_uses_cached_wheel_metadata() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), None, true)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/{WHEEL}.metadata")))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    mount_wheel(&server).await?;
    let cache = Cache::temp()?.init().await?;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let online = client(cache.clone(), &index)?;
    let dist = distribution(&online).await?;
    metadata(&online, &dist).await?;
    let request_count = server.received_requests().await.context("requests")?.len();
    let offline = RegistryClientBuilder::new(
        BaseClientBuilder::default().connectivity(Connectivity::Offline),
        cache,
    )
    .index_locations(IndexLocations::new(
        vec![Index::from_index_url(index)],
        vec![],
        false,
    ))
    .build()?;
    let dist = distribution(&offline).await?;
    assert_eq!(
        metadata(&offline, &dist).await?.version.to_string(),
        "1.0.0"
    );
    assert_eq!(
        server.received_requests().await.context("requests")?.len(),
        request_count
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_reduces_transfer() -> Result<()> {
    let mut transfers = Vec::new();
    for artifactory in [false, true] {
        let server = MockServer::start().await;
        let mut response = simple(&format!("/{WHEEL}"), None, true);
        if artifactory {
            response = response.insert_header("X-JFrog-Version", "Artifactory/7.0.0");
        }
        Mock::given(method("GET"))
            .and(path("/simple/ok/"))
            .respond_with(response)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{WHEEL}.metadata")))
            .respond_with(ResponseTemplate::new(200).set_body_string(METADATA))
            .mount(&server)
            .await;
        mount_wheel(&server).await?;
        let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
        let client = client(Cache::temp()?.init().await?, &index)?;
        let dist = distribution(&client).await?;
        metadata(&client, &dist).await?;
        let requests = server.received_requests().await.context("requests")?;
        let body_bytes: usize = requests
            .iter()
            .filter(|request| request.method == "GET")
            .map(|request| {
                if request.url.path() == format!("/{WHEEL}") {
                    875
                } else if request.url.path() == format!("/{WHEEL}.metadata") {
                    METADATA.len()
                } else {
                    0
                }
            })
            .sum();
        transfers.push((artifactory, requests.len(), body_bytes));
    }
    insta::assert_debug_snapshot!(transfers, @r#"
    [
        (
            false,
            3,
            875,
        ),
        (
            true,
            2,
            47,
        ),
    ]
    "#);
    Ok(())
}
