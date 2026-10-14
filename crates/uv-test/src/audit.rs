//! A local OSV service for tests that expect no vulnerabilities.

use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const QUERY_BATCH_PATH: &str = "/v1/querybatch";

#[derive(Deserialize)]
struct QueryBatch {
    queries: Vec<Query>,
}

#[derive(Deserialize)]
struct Query {
    package: Package,
    version: String,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    ecosystem: String,
}

/// A running audit service that returns one clean result for every queried PyPI package.
///
/// The service requires at least one batch request before it is dropped.
pub struct CleanAuditService {
    server: MockServer,
}

impl CleanAuditService {
    /// Start a clean OSV query-batch endpoint on a local ephemeral port.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(QUERY_BATCH_PATH))
            .respond_with(
                |request: &Request| match request.body_json::<QueryBatch>() {
                    Ok(batch) => {
                        if let Some(query) = batch
                            .queries
                            .iter()
                            .find(|query| query.package.ecosystem != "PyPI")
                        {
                            return ResponseTemplate::new(400).set_body_string(format!(
                                "expected PyPI ecosystem for {}, got {}",
                                query.package.name, query.package.ecosystem
                            ));
                        }
                        ResponseTemplate::new(200).set_body_json(json!({
                            "results": vec![json!({ "vulns": [] }); batch.queries.len()]
                        }))
                    }
                    Err(error) => ResponseTemplate::new(400)
                        .set_body_string(format!("invalid OSV query batch: {error}")),
                },
            )
            .expect(1..)
            .mount(&server)
            .await;
        Self { server }
    }

    /// The base URI to pass to `--service-url`.
    pub fn uri(&self) -> String {
        self.server.uri()
    }

    /// Assert the sequence of queried package/version batches, ignoring order within each batch.
    pub async fn assert_queries(&self, expected: &[&[(&str, &str)]]) {
        let requests = self
            .server
            .received_requests()
            .await
            .expect("audit service records requests");
        assert_eq!(
            requests.len(),
            expected.len(),
            "unexpected audit query count"
        );

        for (index, (request, expected)) in requests.iter().zip(expected).enumerate() {
            assert_eq!(
                (request.method.as_str(), request.url.path()),
                ("POST", QUERY_BATCH_PATH),
                "unexpected audit request {}",
                index + 1,
            );
            let batch = request
                .body_json::<QueryBatch>()
                .expect("audit query contains packages and versions");
            let mut actual: Vec<_> = batch
                .queries
                .iter()
                .map(|query| (query.package.name.as_str(), query.version.as_str()))
                .collect();
            actual.sort_unstable();
            let mut expected = expected.to_vec();
            expected.sort_unstable();
            assert_eq!(
                actual,
                expected,
                "unexpected audit query batch {}",
                index + 1
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{CleanAuditService, QUERY_BATCH_PATH};

    #[tokio::test]
    async fn responds_to_each_query_and_preserves_batch_boundaries() {
        let service = CleanAuditService::start().await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}{QUERY_BATCH_PATH}", service.uri());
        for (packages, expected) in [
            (
                vec![("beta", "2"), ("alpha", "1"), ("beta", "2")],
                json!({ "results": [{ "vulns": [] }, { "vulns": [] }, { "vulns": [] }] }),
            ),
            (
                vec![("gamma", "3")],
                json!({ "results": [{ "vulns": [] }] }),
            ),
        ] {
            let queries: Vec<_> = packages
                .into_iter()
                .map(|(name, version)| {
                    json!({ "package": { "name": name, "ecosystem": "PyPI" }, "version": version })
                })
                .collect();
            let response = client
                .post(&url)
                .json(&json!({ "queries": queries }))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json::<Value>()
                .await
                .unwrap();
            assert_eq!(response, expected);
        }

        service
            .assert_queries(&[
                &[("alpha", "1"), ("beta", "2"), ("beta", "2")],
                &[("gamma", "3")],
            ])
            .await;
    }

    #[tokio::test]
    async fn rejects_a_batch_with_non_pypi_query() {
        let service = CleanAuditService::start().await;
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("{}{QUERY_BATCH_PATH}", service.uri()))
            .json(&json!({ "queries": [
                { "package": { "name": "first", "ecosystem": "PyPI" }, "version": "1" },
                { "package": { "name": "second", "ecosystem": "npm" }, "version": "2" }
            ] }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
}
