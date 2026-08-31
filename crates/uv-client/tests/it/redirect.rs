use std::sync::Arc;

use anyhow::{Context, Result};
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{ClientWithMiddleware, Middleware, Next};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use uv_client::{AuthIntegration, BaseClientBuilder, ExtraMiddleware, RedirectPolicy};
use uv_redacted::DisplaySafeUrl;

#[derive(Clone, Default)]
struct RequestLog(Vec<(&'static str, String)>);

struct RecordRequests(&'static str);

#[async_trait::async_trait]
impl Middleware for RecordRequests {
    async fn handle(
        &self,
        request: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> reqwest_middleware::Result<Response> {
        extensions
            .get_mut::<RequestLog>()
            .expect("caller extensions should reach every middleware")
            .0
            .push((self.0, request.url().path().to_owned()));
        next.run(request, extensions).await
    }
}

/// The standard client must retain caller extensions and run appended middleware on each hop.
#[tokio::test]
async fn redirect_preserves_middleware_and_extensions() -> Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/source"))
        .respond_with(ResponseTemplate::new(303).insert_header("location", "/target"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/target"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let url = DisplaySafeUrl::parse(&format!("{}/source", server.uri()))?;
    let base = BaseClientBuilder::default()
        .redirect(RedirectPolicy::RetriggerMiddleware)
        .retries(0)
        .auth_integration(AuthIntegration::NoAuthMiddleware)
        .extra_middleware(ExtraMiddleware(vec![Arc::new(RecordRequests("base"))]))
        .build()?;
    let client: ClientWithMiddleware = base.for_host(&url).clone();
    let client = reqwest_middleware::ClientBuilder::from_client(client)
        .with(RecordRequests("appended"))
        .build();
    let mut extensions = Extensions::new();
    extensions.insert(RequestLog::default());
    let response = client
        .execute_with_extensions(client.get(url.as_str()).build()?, &mut extensions)
        .await?
        .error_for_status()?;
    assert_eq!(response.url().path(), "/target");
    assert_eq!(
        extensions
            .get::<RequestLog>()
            .context("caller extensions should retain the request log")?
            .0,
        [
            ("base", "/source".to_owned()),
            ("appended", "/source".to_owned()),
            ("base", "/target".to_owned()),
            ("appended", "/target".to_owned()),
        ]
    );
    Ok(())
}

/// Each redirect hop has its own retry budget, without resending successful earlier hops.
#[tokio::test]
async fn redirect_retries_each_hop() -> Result<()> {
    let server = MockServer::start().await;
    for request_path in ["/source", "/target"] {
        Mock::given(method("GET"))
            .and(path(request_path))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("GET"))
        .and(path("/source"))
        .respond_with(ResponseTemplate::new(303).insert_header("location", "/target"))
        .with_priority(6)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/target"))
        .respond_with(ResponseTemplate::new(200))
        .with_priority(6)
        .expect(1)
        .mount(&server)
        .await;

    let url = DisplaySafeUrl::parse(&format!("{}/source", server.uri()))?;
    let client = BaseClientBuilder::default()
        .redirect(RedirectPolicy::RetriggerMiddleware)
        .retries(1)
        .no_retry_delay(true)
        .auth_integration(AuthIntegration::NoAuthMiddleware)
        .build()?;
    let response = client
        .for_host(&url)
        .get(url.as_str())
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(response.url().path(), "/target");
    Ok(())
}
