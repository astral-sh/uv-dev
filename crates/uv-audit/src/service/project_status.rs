//! Auditing for [PEP 792] adverse project statuses.
//!
//! [PEP 792]: https://peps.python.org/pep-0792/

use futures::{StreamExt as _, stream};
use rustc_hash::FxHashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{OnceCell, Semaphore};
use tracing::trace;

use uv_client::{MetadataFormat, RegistryClient};
use uv_configuration::Concurrency;
use uv_distribution_types::{IndexCapabilities, IndexMetadataRef, IndexUrl};
use uv_normalize::PackageName;
use uv_pypi_types::{ProjectStatus as PypiProjectStatus, Status};

use crate::types::{self, AdverseStatus, Finding};

type ProjectStatusCell = Arc<OnceCell<Option<Finding>>>;

/// Audit projects for PEP 792 adverse status markers using a [`RegistryClient`].
pub struct ProjectStatusAudit<'a> {
    client: &'a RegistryClient,
    capabilities: &'a IndexCapabilities,
    concurrency: Concurrency,
    cache: Mutex<FxHashMap<(PackageName, IndexUrl), ProjectStatusCell>>,
}

impl<'a> ProjectStatusAudit<'a> {
    /// Create a new audit session backed by the given [`RegistryClient`].
    pub fn new(
        client: &'a RegistryClient,
        capabilities: &'a IndexCapabilities,
        concurrency: Concurrency,
    ) -> Self {
        Self {
            client,
            capabilities,
            concurrency,
            cache: Mutex::default(),
        }
    }

    /// Query the project-level status of each project on its index.
    ///
    /// Transient per-project query failures (network errors, not-found, offline
    /// without a cache hit) are logged and dropped, on the principle that one
    /// misbehaving index should not invalidate the rest of the audit.
    pub async fn query_batch(&self, projects: &[(&PackageName, IndexUrl)]) -> Vec<Finding> {
        if projects.is_empty() {
            return Vec::new();
        }

        let semaphore = self.concurrency.downloads_semaphore.clone();

        stream::iter(projects)
            .map(|(name, index)| {
                let semaphore = semaphore.clone();
                async move { self.query_one(name, index, semaphore.as_ref()).await }
            })
            .buffer_unordered(self.concurrency.downloads)
            .filter_map(|finding| async move { finding })
            .collect()
            .await
    }

    async fn query_one(
        &self,
        name: &PackageName,
        index: &IndexUrl,
        semaphore: &Semaphore,
    ) -> Option<Finding> {
        let cell = {
            let mut cache = self.cache.lock().expect("project-status cache mutex");
            cache
                .entry((name.clone(), index.clone()))
                .or_default()
                .clone()
        };
        // Share successful lookups, including active projects, across lockfiles. Failed
        // requests leave the cell empty so a later tool can retry the project.
        match cell
            .get_or_try_init(async || self.fetch(name, index, semaphore).await)
            .await
        {
            Ok(finding) => finding.clone(),
            Err(()) => None,
        }
    }

    async fn fetch(
        &self,
        name: &PackageName,
        index: &IndexUrl,
        semaphore: &Semaphore,
    ) -> Result<Option<Finding>, ()> {
        let results = match self
            .client
            .simple_detail(
                name,
                Some(IndexMetadataRef::from(index)),
                self.capabilities,
                semaphore,
            )
            .await
        {
            Ok(results) => results,
            Err(err) => {
                trace!("Skipping project-status check for `{name}`: {err}");
                return Err(());
            }
        };

        let archive = results
            .into_iter()
            .map(|(_, format)| match format {
                MetadataFormat::Simple(archive) => archive,
                MetadataFormat::Flat(_) => {
                    unreachable!("Flat metadata should not be returned by `simple_detail`")
                }
            })
            .next()
            .ok_or(())?;

        let project_status: PypiProjectStatus =
            match rkyv::deserialize::<PypiProjectStatus, rkyv::rancor::Error>(
                archive.project_status(),
            ) {
                Ok(project_status) => project_status,
                Err(err) => {
                    trace!("Failed to read archived project status for `{name}`: {err}");
                    return Err(());
                }
            };

        let Some(status) = to_adverse(project_status.status) else {
            return Ok(None);
        };
        let reason = project_status.reason.map(|reason| reason.to_string());
        Ok(Some(Finding::ProjectStatus(types::ProjectStatus {
            name: name.clone(),
            status,
            reason,
        })))
    }
}

/// Map a PEP 792 [`Status`] to its [`AdverseStatus`] counterpart, if any.
fn to_adverse(status: Status) -> Option<AdverseStatus> {
    match status {
        Status::Active => None,
        Status::Archived => Some(AdverseStatus::Archived),
        Status::Quarantined => Some(AdverseStatus::Quarantined),
        Status::Deprecated => Some(AdverseStatus::Deprecated),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use serde_json::json;
    use uv_cache::Cache;
    use uv_client::{BaseClientBuilder, RegistryClientBuilder};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn response(name: &str, status: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_raw(
                json!({
                    "meta": { "api-version": "1.1" },
                    "name": name,
                    "files": [],
                    "project-status": { "status": status }
                })
                .to_string(),
                "application/vnd.pypi.simple.v1+json",
            )
            .insert_header("Cache-Control", "no-store")
    }

    #[tokio::test]
    async fn shares_successful_project_queries() {
        let server = MockServer::start().await;
        for status in ["active", "archived"] {
            Mock::given(method("GET"))
                .and(path(format!("/simple/{status}/")))
                .respond_with(response(status, status).set_delay(Duration::from_millis(50)))
                .expect(1)
                .mount(&server)
                .await;
        }
        let client =
            RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp().unwrap())
                .build()
                .unwrap();
        let capabilities = IndexCapabilities::default();
        let audit = ProjectStatusAudit::new(&client, &capabilities, Concurrency::default());
        let index = IndexUrl::parse(&format!("{}/simple", server.uri()), None).unwrap();
        let active = PackageName::from_str("active").unwrap();
        let archived = PackageName::from_str("archived").unwrap();
        let projects = [(&active, index.clone()), (&archived, index)];

        let (left, right) =
            tokio::join!(audit.query_batch(&projects), audit.query_batch(&projects));
        let later = audit.query_batch(&projects).await;
        for findings in [left, right, later] {
            assert!(matches!(
                findings.as_slice(),
                [Finding::ProjectStatus(status)]
                    if status.name == archived && status.status == AdverseStatus::Archived
            ));
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn keeps_index_statuses_distinct() {
        let server = MockServer::start().await;
        for (index, status) in [("first", "archived"), ("second", "deprecated")] {
            Mock::given(method("GET"))
                .and(path(format!("/{index}/shared/")))
                .respond_with(response("shared", status))
                .expect(1)
                .mount(&server)
                .await;
        }
        let client =
            RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp().unwrap())
                .build()
                .unwrap();
        let capabilities = IndexCapabilities::default();
        let audit = ProjectStatusAudit::new(&client, &capabilities, Concurrency::default());
        let name = PackageName::from_str("shared").unwrap();
        let projects = [
            (
                &name,
                IndexUrl::parse(&format!("{}/first", server.uri()), None).unwrap(),
            ),
            (
                &name,
                IndexUrl::parse(&format!("{}/second", server.uri()), None).unwrap(),
            ),
        ];

        for _ in 0..2 {
            let findings = audit.query_batch(&projects).await;
            assert_eq!(findings.len(), 2);
            assert!(findings.iter().any(|finding| matches!(finding,
                Finding::ProjectStatus(status) if status.status == AdverseStatus::Archived)));
            assert!(findings.iter().any(|finding| matches!(finding,
                Finding::ProjectStatus(status) if status.status == AdverseStatus::Deprecated)));
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn retries_skipped_project_queries() {
        let server = MockServer::start().await;
        let requests = Arc::new(AtomicUsize::new(0));
        Mock::given(method("GET"))
            .and(path("/simple/shared/"))
            .respond_with({
                let requests = Arc::clone(&requests);
                move |_: &wiremock::Request| {
                    if requests.fetch_add(1, Ordering::SeqCst) == 0 {
                        ResponseTemplate::new(404).insert_header("Cache-Control", "no-store")
                    } else {
                        response("shared", "archived")
                    }
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        let client =
            RegistryClientBuilder::new(BaseClientBuilder::default(), Cache::temp().unwrap())
                .build()
                .unwrap();
        let capabilities = IndexCapabilities::default();
        let audit = ProjectStatusAudit::new(&client, &capabilities, Concurrency::default());
        let name = PackageName::from_str("shared").unwrap();
        let index = IndexUrl::parse(&format!("{}/simple", server.uri()), None).unwrap();
        let projects = [(&name, index)];

        assert!(audit.query_batch(&projects).await.is_empty());
        assert!(matches!(audit.query_batch(&projects).await.as_slice(),
            [Finding::ProjectStatus(status)] if status.status == AdverseStatus::Archived));
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.verify().await;
    }
}
