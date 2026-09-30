use tokio::sync::Semaphore;
use tracing::debug;

use uv_client::{MetadataFormat, RegistryClient, VersionFiles};
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{
    File, IndexCapabilities, IndexLocations, IndexMetadataRef, IndexUrl, RequiresPython,
};
use uv_normalize::PackageName;
use uv_platform_tags::Tags;
use uv_resolver::{ExcludeNewer, Prerelease, PrereleaseMode};
use uv_warnings::warn_user_once;

/// A client to fetch the latest version of a package from an index.
///
/// The returned distribution is guaranteed to be compatible with the provided tags and Python
/// requirement (if specified).
#[derive(Debug, Clone)]
pub(crate) struct LatestClient<'env> {
    pub(crate) client: &'env RegistryClient,
    pub(crate) capabilities: &'env IndexCapabilities,
    pub(crate) prerelease: &'env Prerelease,
    pub(crate) exclude_newer: &'env ExcludeNewer,
    pub(crate) index_locations: &'env IndexLocations,
    pub(crate) tags: Option<&'env Tags>,
    pub(crate) requires_python: Option<&'env RequiresPython>,
}

impl LatestClient<'_> {
    fn effective_exclude_newer(
        &self,
        package: &PackageName,
        index: &IndexUrl,
    ) -> Option<jiff::Timestamp> {
        self.exclude_newer
            .exclude_newer_package_for_index(package, self.index_locations.exclude_newer_for(index))
    }

    fn consider_candidate(
        &self,
        package: &PackageName,
        filename: &DistFilename,
        file: &File,
        exclude_newer: Option<&jiff::Timestamp>,
    ) -> bool {
        // Respect any exclude-newer cutoffs that were provided.
        if let Some(exclude_newer) = exclude_newer {
            match file.upload_time_utc_ms.as_ref() {
                Some(&upload_time) if upload_time >= exclude_newer.as_millisecond() => {
                    return false;
                }
                None => {
                    warn_user_once!(
                        "{} is missing an upload date, but user provided: {}",
                        file.filename,
                        exclude_newer
                    );
                }
                _ => {}
            }
        }

        // Unless explicitly allowed, skip pre-release artifacts.
        let prerelease = self.prerelease.mode(package);
        if !filename.version().is_stable() && !matches!(prerelease, PrereleaseMode::Allow) {
            return false;
        }

        // Avoid yanked or otherwise withdrawn files.
        if file
            .yanked
            .as_ref()
            .is_some_and(|yanked| yanked.is_yanked())
        {
            return false;
        }

        // Enforce the interpreter's `Requires-Python` constraints.
        if let Some(requires_python) = self.requires_python
            && file
                .requires_python
                .as_ref()
                .is_some_and(|file_requires_python| {
                    !requires_python.is_contained_by(file_requires_python)
                })
        {
            return false;
        }

        // Skip wheels that aren't compatible with the current platform.
        if let DistFilename::WheelFilename(filename) = filename
            && self
                .tags
                .is_some_and(|tags| !filename.compatibility(tags).is_compatible())
        {
            return false;
        }

        true
    }

    /// Find the latest version of a package from an index.
    pub(crate) async fn find_latest(
        &self,
        package: &PackageName,
        index: Option<&IndexUrl>,
        download_concurrency: &Semaphore,
    ) -> Result<Option<DistFilename>, uv_client::Error> {
        debug!("Fetching latest version of: `{package}`");

        let mut latest: Option<DistFilename> = None;

        let mut update_latest = |candidate: DistFilename| {
            // Prefer higher versions, and prefer wheels over sdists at parity.
            if latest.as_ref().is_none_or(|current| {
                candidate.version() > current.version()
                    || (candidate.version() == current.version()
                        && matches!(candidate, DistFilename::WheelFilename(_))
                        && matches!(current, DistFilename::SourceDistFilename(_)))
            }) {
                latest = Some(candidate);
            }
        };

        let simple = self.client.simple_detail(
            package,
            index.map(IndexMetadataRef::from),
            self.capabilities,
            download_concurrency,
        );
        let find_links = async {
            if index.is_none() {
                self.client
                    .find_links_entries(package, download_concurrency)
                    .await
            } else {
                Ok(Vec::new())
            }
        };
        tokio::pin!(simple, find_links);

        // Both sources contribute available versions. Poll them together while giving Simple
        // API failures priority and cancelling outstanding find-links work on those failures.
        let (archives, find_links_result) = tokio::select! {
            archives = &mut simple => (archives, None),
            entries = &mut find_links => (simple.await, Some(entries)),
        };
        let archives = match archives {
            Ok(archives) => archives,
            Err(err)
                if matches!(
                    err.kind(),
                    uv_client::ErrorKind::RemotePackageNotFound(_)
                        | uv_client::ErrorKind::NoIndex(_)
                        | uv_client::ErrorKind::Offline(_)
                ) =>
            {
                Vec::new()
            }
            Err(err) => return Err(err),
        };

        for (index, archive) in archives {
            let exclude_newer = self.effective_exclude_newer(package, index);

            match archive {
                MetadataFormat::Simple(archive) => {
                    for datum in archive.iter().rev() {
                        let files =
                            rkyv::deserialize::<VersionFiles, rkyv::rancor::Error>(&datum.files)
                                .expect("archived version files always deserializes");

                        for (filename, file) in files.all(package) {
                            if self.consider_candidate(
                                package,
                                &filename,
                                &file,
                                exclude_newer.as_ref(),
                            ) {
                                update_latest(filename);
                            }
                        }
                    }
                }
                MetadataFormat::Flat(entries) => {
                    for entry in entries {
                        let (filename, file, _) = entry.into_parts();
                        if self.consider_candidate(
                            package,
                            &filename,
                            &file,
                            exclude_newer.as_ref(),
                        ) {
                            update_latest(filename);
                        }
                    }
                }
            }
        }

        let find_links_entries = match find_links_result {
            Some(entries) => entries?,
            None => find_links.await?,
        };
        for entry in find_links_entries {
            let (filename, file, index) = entry.into_parts();
            let exclude_newer = self.effective_exclude_newer(package, &index);
            if self.consider_candidate(package, &filename, &file, exclude_newer.as_ref()) {
                update_latest(filename);
            }
        }

        Ok(latest)
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;

    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::service::service_fn;
    use hyper_util::rt::TokioIo;
    use tokio::net::TcpListener;
    use tokio::sync::{Notify, Semaphore};
    use uv_cache::Cache;
    use uv_client::{BaseClientBuilder, RegistryClientBuilder};
    use uv_distribution_filename::DistFilename;
    use uv_distribution_types::{Index, IndexCapabilities, IndexLocations, IndexUrl};
    use uv_normalize::PackageName;
    use uv_resolver::{ExcludeNewer, Prerelease};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::LatestClient;

    async fn find_latest(
        simple: &str,
        flat: &str,
    ) -> Result<Option<DistFilename>, uv_client::Error> {
        let locations = IndexLocations::new(
            vec![Index::from_index_url(IndexUrl::from_str(simple).unwrap())],
            vec![Index::from_find_links(IndexUrl::from_str(flat).unwrap())],
            false,
        );
        let registry = RegistryClientBuilder::new(
            BaseClientBuilder::default().retries(0),
            Cache::temp().unwrap().init().await.unwrap(),
        )
        .index_locations(locations.clone())
        .build()
        .unwrap();
        LatestClient {
            client: &registry,
            capabilities: &IndexCapabilities::default(),
            prerelease: &Prerelease::default(),
            exclude_newer: &ExcludeNewer::default(),
            index_locations: &locations,
            tags: None,
            requires_python: None,
        }
        .find_latest(
            &PackageName::from_str("example").unwrap(),
            None,
            &Semaphore::new(2),
        )
        .await
    }

    #[tokio::test]
    async fn latest_lookup_fetches_both_sources_concurrently() -> anyhow::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let flat_started = Arc::new(Notify::new());
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let flat_started = Arc::clone(&flat_started);
                tokio::spawn(async move {
                    let service = service_fn(
                        move |request: hyper::Request<hyper::body::Incoming>| {
                            let flat_started = Arc::clone(&flat_started);
                            async move {
                                let (body, content_type) = if request.uri().path() == "/flat" {
                                    flat_started.notify_one();
                                    (
                                        b"<a href='example-2.0-py3-none-any.whl'>example</a>"
                                            .as_slice(),
                                        "text/html",
                                    )
                                } else {
                                    flat_started.notified().await;
                                    (br#"{"meta":{"api-version":"1.0"},"name":"example","files":[]}"#.as_slice(), "application/vnd.pypi.simple.v1+json")
                                };
                                Ok::<_, Infallible>(
                                    hyper::Response::builder()
                                        .header(http::header::CONTENT_TYPE, content_type)
                                        .body(Full::new(Bytes::from_static(body)))
                                        .unwrap(),
                                )
                            }
                        },
                    );
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await
                });
            }
        });
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            find_latest(
                &format!("http://{address}/simple"),
                &format!("http://{address}/flat"),
            ),
        )
        .await;
        server.abort();
        assert_eq!(result??.unwrap().version().to_string(), "2.0");
        Ok(())
    }

    #[tokio::test]
    async fn latest_lookup_keeps_index_failure_priority() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/example/"))
            .respond_with(ResponseTemplate::new(500).set_delay(Duration::from_millis(50)))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/flat"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let error = find_latest(
            &format!("{}/simple", server.uri()),
            &format!("{}/flat", server.uri()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("/simple/example/"), "{error}");
        Ok(())
    }

    #[tokio::test]
    async fn latest_lookup_cancels_find_links_after_index_failure() -> anyhow::Result<()> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/simple/example/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/flat"))
            .respond_with(ResponseTemplate::new(500).set_delay(Duration::from_secs(30)))
            .mount(&server)
            .await;
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            find_latest(
                &format!("{}/simple", server.uri()),
                &format!("{}/flat", server.uri()),
            ),
        )
        .await?
        .unwrap_err();
        assert!(error.to_string().contains("/simple/example/"), "{error}");
        Ok(())
    }
}
