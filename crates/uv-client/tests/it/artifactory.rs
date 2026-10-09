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
use uv_pypi_types::{HashDigests, ResolutionMetadata};
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
async fn artifactory_metadata_discovers_unadvertised_html() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), None, false)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let cache = Cache::temp()?.init().await?;
    let first = client(cache.clone(), &index)?;
    let BuiltDist::Registry(dist) = distribution(&first).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Unadvertised
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_discovers_and_caches_unadvertised_json() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), None, true)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let cache = Cache::temp()?.init().await?;
    let first = client(cache.clone(), &index)?;
    let BuiltDist::Registry(dist) = distribution(&first).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Unadvertised
    );
    // A new client must recover the capability from the cached Simple response.
    let second = client(cache, &index)?;
    let BuiltDist::Registry(dist) = distribution(&second).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Unadvertised
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_respects_explicit_false() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), Some(false), true)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let cache = Cache::temp()?.init().await?;
    let first = client(cache.clone(), &index)?;
    let BuiltDist::Registry(dist) = distribution(&first).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Unavailable
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_respects_advertised_sidecar() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), Some(true), false)
                .insert_header("X-JFrog-Version", "Artifactory/7.0.0"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let cache = Cache::temp()?.init().await?;
    let first = client(cache.clone(), &index)?;
    let BuiltDist::Registry(dist) = distribution(&first).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Available(HashDigests::empty())
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_ignores_other_products() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(&format!("/{WHEEL}"), None, true)
                .insert_header("X-JFrog-Version", "Other/7.0.0"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let cache = Cache::temp()?.init().await?;
    let first = client(cache.clone(), &index)?;
    let BuiltDist::Registry(dist) = distribution(&first).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Unavailable
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_requires_product_header() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(simple(&format!("/{WHEEL}"), None, false))
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let cache = Cache::temp()?.init().await?;
    let first = client(cache.clone(), &index)?;
    let BuiltDist::Registry(dist) = distribution(&first).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Unavailable
    );
    Ok(())
}

#[tokio::test]
async fn advertised_metadata_does_not_require_product_header() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(simple(&format!("/{WHEEL}"), Some(true), true))
        .expect(1)
        .mount(&server)
        .await;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let cache = Cache::temp()?.init().await?;
    let first = client(cache.clone(), &index)?;
    let BuiltDist::Registry(dist) = distribution(&first).await? else {
        anyhow::bail!("expected registry distribution");
    };
    assert_eq!(
        dist.best_wheel().file.dist_info_metadata,
        DistInfoMetadata::Available(HashDigests::empty())
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_rejects_cross_origin_wheel() -> Result<()> {
    let origin = MockServer::start().await;
    let other = MockServer::start().await;
    let index = format!("{}/simple", origin.uri());
    let target = format!("{}/{WHEEL}", other.uri());
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
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_rejects_cross_origin_index_redirect() -> Result<()> {
    let origin = MockServer::start().await;
    let other = MockServer::start().await;
    let index = format!("{}/redirect", origin.uri());
    let target = format!("{}/{WHEEL}", origin.uri());
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
    Ok(())
}

#[tokio::test]
async fn artifactory_unadvertised_metadata_keeps_dependencies_and_query() -> Result<()> {
    let sidecar = METADATA.replace("\n\n", "\nRequires-Dist: other==2\n\n");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(
                &format!("/{WHEEL}?download=1#sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                None,
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
    let first_client = client(cache.clone(), &index)?;
    let dist = distribution(&first_client).await?;
    let first_metadata = metadata(&first_client, &dist).await?;
    assert_eq!(first_metadata.name.as_ref(), "ok");
    assert_eq!(first_metadata.version.to_string(), "1.0.0");
    assert_eq!(first_metadata.requires_dist.len(), 1);
    let second_client = client(cache.clone(), &index)?;
    let dist = distribution(&second_client).await?;
    let second_metadata = metadata(&second_client, &dist).await?;
    assert_eq!(second_metadata.name.as_ref(), "ok");
    assert_eq!(second_metadata.version.to_string(), "1.0.0");
    assert_eq!(second_metadata.requires_dist.len(), 1);
    assert_eq!(
        server.received_requests().await.context("requests")?.len(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_advertised_metadata_keeps_dependencies_and_query() -> Result<()> {
    let sidecar = METADATA.replace("\n\n", "\nRequires-Dist: other==2\n\n");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/simple/ok/"))
        .respond_with(
            simple(
                &format!("/{WHEEL}?download=1#sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
                Some(true),
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
    let first_client = client(cache.clone(), &index)?;
    let dist = distribution(&first_client).await?;
    let first_metadata = metadata(&first_client, &dist).await?;
    assert_eq!(first_metadata.name.as_ref(), "ok");
    assert_eq!(first_metadata.version.to_string(), "1.0.0");
    assert_eq!(first_metadata.requires_dist.len(), 1);
    let second_client = client(cache.clone(), &index)?;
    let dist = distribution(&second_client).await?;
    let second_metadata = metadata(&second_client, &dist).await?;
    assert_eq!(second_metadata.name.as_ref(), "ok");
    assert_eq!(second_metadata.version.to_string(), "1.0.0");
    assert_eq!(second_metadata.requires_dist.len(), 1);
    assert_eq!(
        server.received_requests().await.context("requests")?.len(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_falls_back_on_missing_sidecar() -> Result<()> {
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
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let client = client(Cache::temp()?.init().await?, &index)?;
    let dist = distribution(&client).await?;
    let downloaded = metadata(&client, &dist).await?;
    assert_eq!(downloaded.name.as_ref(), "ok");
    assert_eq!(downloaded.version.to_string(), "1.0.0");
    assert!(downloaded.requires_dist.is_empty());

    let cached = metadata(&client, &dist).await?;
    assert_eq!(cached.name.as_ref(), "ok");
    assert_eq!(cached.version.to_string(), "1.0.0");
    assert!(cached.requires_dist.is_empty());
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_falls_back_on_unsupported_sidecar() -> Result<()> {
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
        .respond_with(ResponseTemplate::new(405))
        .expect(1)
        .mount(&server)
        .await;
    mount_wheel(&server).await?;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let client = client(Cache::temp()?.init().await?, &index)?;
    let dist = distribution(&client).await?;
    let downloaded = metadata(&client, &dist).await?;
    assert_eq!(downloaded.name.as_ref(), "ok");
    assert_eq!(downloaded.version.to_string(), "1.0.0");
    assert!(downloaded.requires_dist.is_empty());

    let cached = metadata(&client, &dist).await?;
    assert_eq!(cached.name.as_ref(), "ok");
    assert_eq!(cached.version.to_string(), "1.0.0");
    assert!(cached.requires_dist.is_empty());
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_falls_back_on_malformed_sidecar() -> Result<()> {
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
        .respond_with(ResponseTemplate::new(200).set_body_string("not metadata"))
        .expect(1)
        .mount(&server)
        .await;
    mount_wheel(&server).await?;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let client = client(Cache::temp()?.init().await?, &index)?;
    let dist = distribution(&client).await?;
    let downloaded = metadata(&client, &dist).await?;
    assert_eq!(downloaded.name.as_ref(), "ok");
    assert_eq!(downloaded.version.to_string(), "1.0.0");
    assert!(downloaded.requires_dist.is_empty());

    let cached = metadata(&client, &dist).await?;
    assert_eq!(cached.name.as_ref(), "ok");
    assert_eq!(cached.version.to_string(), "1.0.0");
    assert!(cached.requires_dist.is_empty());
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_falls_back_on_sidecar_name_mismatch() -> Result<()> {
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
            ResponseTemplate::new(200).set_body_string(METADATA.replace("Name: ok", "Name: other")),
        )
        .expect(1)
        .mount(&server)
        .await;
    mount_wheel(&server).await?;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let client = client(Cache::temp()?.init().await?, &index)?;
    let dist = distribution(&client).await?;
    let downloaded = metadata(&client, &dist).await?;
    assert_eq!(downloaded.name.as_ref(), "ok");
    assert_eq!(downloaded.version.to_string(), "1.0.0");
    assert!(downloaded.requires_dist.is_empty());

    let cached = metadata(&client, &dist).await?;
    assert_eq!(cached.name.as_ref(), "ok");
    assert_eq!(cached.version.to_string(), "1.0.0");
    assert!(cached.requires_dist.is_empty());
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_falls_back_on_sidecar_version_mismatch() -> Result<()> {
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
                .set_body_string(METADATA.replace("Version: 1.0.0", "Version: 2.0.0")),
        )
        .expect(1)
        .mount(&server)
        .await;
    mount_wheel(&server).await?;
    let index = IndexUrl::from_str(&format!("{}/simple", server.uri()))?;
    let client = client(Cache::temp()?.init().await?, &index)?;
    let dist = distribution(&client).await?;
    let downloaded = metadata(&client, &dist).await?;
    assert_eq!(downloaded.name.as_ref(), "ok");
    assert_eq!(downloaded.version.to_string(), "1.0.0");
    assert!(downloaded.requires_dist.is_empty());

    let cached = metadata(&client, &dist).await?;
    assert_eq!(cached.name.as_ref(), "ok");
    assert_eq!(cached.version.to_string(), "1.0.0");
    assert!(cached.requires_dist.is_empty());
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_unauthorized_is_not_optional() -> Result<()> {
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
        .respond_with(ResponseTemplate::new(401))
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
    Ok(())
}

#[tokio::test]
async fn artifactory_metadata_forbidden_is_not_optional() -> Result<()> {
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
        .respond_with(ResponseTemplate::new(403))
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
