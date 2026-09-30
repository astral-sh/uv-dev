//! Mock responses for the OSV vulnerability service.

use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

/// Expect one batch query for the given PyPI packages and return no vulnerabilities.
///
/// The [OSV batch API] identifies queries by result position. Its [response implementation]
/// emits one result for each query in the request, including queries without vulnerabilities.
///
/// [OSV batch API]: https://github.com/google/osv.dev/blob/496c344c3ef1ef52144d2ac48969a8523e34061d/docs/api/post-v1-querybatch.md#L8-L9
/// [response implementation]: https://github.com/google/osv.dev/blob/496c344c3ef1ef52144d2ac48969a8523e34061d/go/internal/api/query_affected.go#L302-L324
pub fn mock_clean_batch(packages: &[(&str, &str)]) -> Mock {
    let queries = packages
        .iter()
        .map(|(name, version)| {
            json!({
                "package": { "name": name, "ecosystem": "PyPI" },
                "version": version,
            })
        })
        .collect::<Vec<_>>();
    let results = vec![json!({ "vulns": [] }); packages.len()];

    Mock::given(method("POST"))
        .and(path("/v1/querybatch"))
        .and(body_json(json!({ "queries": queries })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": results })))
        .expect(1)
}
