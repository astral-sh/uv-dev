use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mount an OSV advisory and a single-query batch response referring to it.
pub async fn mount_advisory(server: &MockServer, id: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"vulns": [{"id": id}]}]
        })))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("/v1/vulns/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": id,
            "modified": "2026-01-01T00:00:00Z",
        })))
        .mount(server)
        .await;
}
