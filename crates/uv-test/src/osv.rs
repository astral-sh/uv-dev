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

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use reqwest::Client;
    use serde_json::{Value, json};
    use wiremock::MockServer;

    use super::mount_advisory;

    #[tokio::test]
    async fn advisory_responses() -> Result<()> {
        let server = MockServer::start().await;
        mount_advisory(&server, "MAL-2026-1234").await;

        let client = Client::new();
        let response = client
            .get(format!("{}/v1/vulns/MAL-2026-1234", server.uri()))
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await?;
        assert_eq!(
            response,
            json!({
                "id": "MAL-2026-1234",
                "modified": "2026-01-01T00:00:00Z",
            })
        );

        Ok(())
    }
}
