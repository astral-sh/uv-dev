use reqwest::Method;
use uv_cache::{Cache, CacheEntry};
use uv_client::{BaseClientBuilder, CacheControl, CachedClient, DataWithCachePolicy, ErrorKind};
use uv_redacted::DisplaySafeUrl;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn reject_overflowing_cache_policy_length() {
    let error = DataWithCachePolicy::from_reader(&[u8::MAX; 8][..]).unwrap_err();

    assert!(matches!(error.kind(), ErrorKind::ArchiveRead(_)));
}

#[tokio::test]
async fn read_cached_representation_matches_request() -> anyhow::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/record"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("cache-control", "max-age=0")
                .insert_header("vary", "x-cache-variant")
                .set_body_json("cached representation"),
        )
        .mount(&server)
        .await;

    let cache = Cache::temp()?;
    let entry = CacheEntry::new(cache.root(), "response.msgpack");
    let client = CachedClient::new(BaseClientBuilder::default().build()?);
    let url = DisplaySafeUrl::parse(&format!("{}/record", server.uri()))?;
    let request = |method, url: &DisplaySafeUrl, variant| {
        client
            .uncached()
            .for_host(url)
            .raw_client()
            .request(method, url.as_ref())
            .header("x-cache-variant", variant)
            .build()
    };
    let payload: String = client
        .get_serde_with_retry(
            request(Method::GET, &url, "first")?,
            &entry,
            CacheControl::None,
            async |response| response.json::<String>().await,
        )
        .await
        .expect("cached response");
    assert_eq!(payload, "cached representation");

    let other = DisplaySafeUrl::parse(&format!("{}/other", server.uri()))?;
    let mut cached = Vec::new();
    for (method, url, variant) in [
        (Method::GET, &url, "first"),
        (Method::HEAD, &url, "first"),
        (Method::POST, &url, "first"),
        (Method::GET, &url, "second"),
        (Method::GET, &other, "first"),
    ] {
        cached.push(
            client
                .read_cached_serde::<String>(&request(method, url, variant)?, &entry)
                .await,
        );
    }
    insta::assert_debug_snapshot!(cached, @r#"
    [
        Some(
            "cached representation",
        ),
        Some(
            "cached representation",
        ),
        None,
        None,
        None,
    ]
    "#);
    assert_eq!(
        server
            .received_requests()
            .await
            .expect("recorded requests")
            .len(),
        1
    );
    Ok(())
}
