use std::sync::Arc;

use tokio::sync::{mpsc::Sender, oneshot};

use uv_distribution_types::{
    Dist, DistributionId, Identifier, IndexMetadata, IndexUrl, Name, ResolutionRecorder,
    ResolvedDistRef,
};
use uv_normalize::PackageName;
use uv_once_map::Registration;
use uv_pep440::Version;

use crate::pubgrub::Range;
use crate::resolver::index::FxRegisteredEntry;
use crate::resolver::{InMemoryIndex, MetadataResponse, Request, Response, VersionsResponse};
use crate::{PythonRequirement, ResolveError};

/// A blocking solver-side handle for requesting metadata from the asynchronous fetcher.
#[derive(Clone)]
pub(crate) struct MetadataRequests {
    index: InMemoryIndex,
    sender: Sender<RequestTask>,
    recorder: Option<ResolutionRecorder>,
    /// Negative lookup results belong to one optional trial, including all of its fork states.
    speculative: Option<InMemoryIndex>,
}

/// An ordinary request is published by the fetcher. A speculative request returns its result to
/// the solver first, so a failed optional lookup cannot stop the shared fetcher or occupy a cache
/// entry needed by a later ordinary request.
pub(crate) struct RequestTask {
    pub(super) request: Request,
    pub(super) response: Option<oneshot::Sender<Result<Option<Response>, ResolveError>>>,
}

impl From<Request> for RequestTask {
    fn from(request: Request) -> Self {
        Self {
            request,
            response: None,
        }
    }
}

/// A distribution request whose cache identity is derived from the requested distribution.
pub(crate) enum MetadataRequest<'a> {
    Dist(Dist),
    Resolved(ResolvedDistRef<'a>),
}

impl MetadataRequest<'_> {
    fn id(&self) -> DistributionId {
        match self {
            Self::Dist(dist) => dist.distribution_id(),
            Self::Resolved(dist) => dist.distribution_id(),
        }
    }

    fn into_request(self) -> Request {
        match self {
            Self::Dist(dist) => Request::Dist(dist),
            Self::Resolved(dist) => Request::from(dist),
        }
    }
}

impl Name for MetadataRequest<'_> {
    fn name(&self) -> &PackageName {
        match self {
            Self::Dist(dist) => dist.name(),
            Self::Resolved(dist) => dist.name(),
        }
    }
}

/// A registered version-list request, bound to its package and index scope.
pub(crate) enum PendingVersions<'index> {
    Implicit(FxRegisteredEntry<'index, PackageName, Arc<VersionsResponse>>),
    Explicit(FxRegisteredEntry<'index, (PackageName, IndexUrl), Arc<VersionsResponse>>),
}

impl PendingVersions<'_> {
    pub(crate) fn wait(self) -> Arc<VersionsResponse> {
        match self {
            Self::Implicit(entry) => entry.wait_blocking(),
            Self::Explicit(entry) => entry.wait_blocking(),
        }
    }
}

/// A distribution whose metadata was registered or supplied before resolution.
///
/// Selected pins retain a registered entry that prevents cache removal while borrowed.
#[derive(Clone, Debug)]
pub(crate) struct RegisteredMetadata<'index>(
    FxRegisteredEntry<'index, DistributionId, Arc<MetadataResponse>>,
);

impl RegisteredMetadata<'_> {
    pub(crate) fn id(&self) -> &DistributionId {
        self.0.key()
    }

    pub(crate) fn wait(&self) -> Arc<MetadataResponse> {
        self.0.wait_blocking()
    }
}

impl MetadataRequests {
    pub(crate) fn new(
        index: InMemoryIndex,
        sender: Sender<RequestTask>,
        recorder: Option<ResolutionRecorder>,
    ) -> Self {
        Self {
            index,
            sender,
            recorder,
            speculative: None,
        }
    }

    /// Return a handle whose failed requests affect only the caller's optional resolution.
    pub(crate) fn speculative(&self) -> Self {
        Self {
            speculative: Some(self.speculative.clone().unwrap_or_default()),
            ..self.clone()
        }
    }

    pub(crate) fn is_speculative(&self) -> bool {
        self.speculative.is_some()
    }

    fn request_speculative(&self, request: Request) -> Result<(), ResolveError> {
        let request = match request {
            request @ (Request::Package(..)
            | Request::Dist(Dist::Built(_))
            | Request::Installed(_)) => request,
            // Source metadata builds can start their own ordinary resolver using the build
            // context's shared index. That nested resolver is not isolated by this handle.
            Request::Dist(Dist::Source(_)) => {
                return Err(uv_distribution::Error::NoBuild.into());
            }
            // Optional work must not enqueue an ordinary asynchronous prefetch.
            Request::Prefetch(..) => return Ok(()),
        };
        let description = request.to_string();
        let (response, receiver) = oneshot::channel();
        self.sender.blocking_send(RequestTask {
            request,
            response: Some(response),
        })?;
        receiver
            .blocking_recv()
            .map_err(|_| ResolveError::ChannelClosed)??
            .ok_or_else(|| ResolveError::UnregisteredTask(description))?
            .publish_speculative(
                &self.index,
                self.speculative
                    .as_ref()
                    .expect("a speculative request has a trial index"),
            )
    }

    /// Schedule a package version request without retaining a handle.
    pub(crate) fn enqueue_package(
        &self,
        name: &PackageName,
        index: Option<&IndexMetadata>,
    ) -> Result<(), ResolveError> {
        if let Some(recorder) = &self.recorder {
            recorder.exclude_newer(name);
        }
        if let Some(speculative) = &self.speculative {
            let cached = if let Some(index) = index {
                let key = (name.clone(), index.url().clone());
                self.index
                    .explicit()
                    .get(&key)
                    .or_else(|| speculative.explicit().get(&key))
            } else {
                self.index
                    .implicit()
                    .get(name)
                    .or_else(|| speculative.implicit().get(name))
            };
            if cached.is_none() {
                self.request_speculative(Request::Package(name.clone(), index.cloned()))?;
            }
            return Ok(());
        }
        let registered = if let Some(index) = index {
            self.index
                .explicit()
                .register((name.clone(), index.url().clone()))
        } else {
            self.index.implicit().register(name.clone())
        };
        if registered {
            self.sender
                .blocking_send(Request::Package(name.clone(), index.cloned()).into())?;
        }
        Ok(())
    }

    /// Request the versions of a package once for the selected index scope.
    pub(crate) fn request_package(
        &self,
        name: &PackageName,
        index: Option<&IndexMetadata>,
    ) -> Result<PendingVersions<'_>, ResolveError> {
        if let Some(recorder) = &self.recorder {
            recorder.exclude_newer(name);
        }
        if let Some(speculative) = &self.speculative {
            if let Some(index) = index {
                let key = (name.clone(), index.url().clone());
                if let Some(entry) = self.index.explicit().get_registered(key.clone()) {
                    return Ok(PendingVersions::Explicit(entry));
                }
                if let Some(entry) = speculative.explicit().get_registered(key.clone()) {
                    return Ok(PendingVersions::Explicit(entry));
                }
                self.request_speculative(Request::Package(name.clone(), Some(index.clone())))?;
                if let Some(entry) = self.index.explicit().get_registered(key.clone()) {
                    return Ok(PendingVersions::Explicit(entry));
                }
                return speculative
                    .explicit()
                    .get_registered(key)
                    .map(PendingVersions::Explicit)
                    .ok_or_else(|| ResolveError::UnregisteredTask(name.to_string()));
            }

            if let Some(entry) = self.index.implicit().get_registered(name.clone()) {
                return Ok(PendingVersions::Implicit(entry));
            }
            if let Some(entry) = speculative.implicit().get_registered(name.clone()) {
                return Ok(PendingVersions::Implicit(entry));
            }
            self.request_speculative(Request::Package(name.clone(), None))?;
            if let Some(entry) = self.index.implicit().get_registered(name.clone()) {
                return Ok(PendingVersions::Implicit(entry));
            }
            return speculative
                .implicit()
                .get_registered(name.clone())
                .map(PendingVersions::Implicit)
                .ok_or_else(|| ResolveError::UnregisteredTask(name.to_string()));
        }

        if let Some(index) = index {
            let entry = match self
                .index
                .explicit()
                .register_entry((name.clone(), index.url().clone()))
            {
                Registration::New(entry) => {
                    self.sender
                        .blocking_send(Request::Package(name.clone(), Some(index.clone())).into())?;
                    entry
                }
                Registration::Existing(entry) => entry,
            };
            Ok(PendingVersions::Explicit(entry))
        } else {
            let entry = match self.index.implicit().register_entry(name.clone()) {
                Registration::New(entry) => {
                    self.sender
                        .blocking_send(Request::Package(name.clone(), None).into())?;
                    entry
                }
                Registration::Existing(entry) => entry,
            };
            Ok(PendingVersions::Implicit(entry))
        }
    }

    /// Schedule metadata retrieval without retaining or cloning its cache identity.
    pub(crate) fn enqueue_metadata(
        &self,
        request: MetadataRequest<'_>,
    ) -> Result<(), ResolveError> {
        if let Some(recorder) = &self.recorder {
            recorder.dependency_metadata(request.name());
        }
        if let Some(speculative) = &self.speculative {
            let id = request.id();
            if self.index.distributions().get_registered(id.clone()).is_none()
                && speculative
                    .distributions()
                    .get_registered(id)
                    .is_none()
            {
                self.request_speculative(request.into_request())?;
            }
            return Ok(());
        }
        if self.index.distributions().register(request.id()) {
            self.sender.blocking_send(request.into_request().into())?;
        }
        Ok(())
    }

    /// Request distribution metadata once, validating and constructing only new requests.
    ///
    /// Metadata already fetched or scheduled by another path does not need a new request.
    pub(crate) fn request_metadata(
        &self,
        request: MetadataRequest<'_>,
        validate: impl FnOnce(&MetadataRequest<'_>) -> Result<(), ResolveError>,
    ) -> Result<RegisteredMetadata<'_>, ResolveError> {
        if let Some(recorder) = &self.recorder {
            recorder.dependency_metadata(request.name());
        }
        if let Some(speculative) = &self.speculative {
            let id = request.id();
            if let Some(entry) = self.index.distributions().get_registered(id.clone()) {
                return Ok(RegisteredMetadata(entry));
            }
            if let Some(entry) = speculative.distributions().get_registered(id.clone()) {
                return Ok(RegisteredMetadata(entry));
            }
            validate(&request)?;
            let request = request.into_request();
            let description = request.to_string();
            self.request_speculative(request)?;
            if let Some(entry) = self.index.distributions().get_registered(id.clone()) {
                return Ok(RegisteredMetadata(entry));
            }
            return speculative
                .distributions()
                .get_registered(id)
                .map(RegisteredMetadata)
                .ok_or_else(|| ResolveError::UnregisteredTask(description));
        }

        let entry = match self.index.distributions().register_entry(request.id()) {
            Registration::New(entry) => {
                validate(&request)?;
                self.sender.blocking_send(request.into_request().into())?;
                entry
            }
            Registration::Existing(entry) => entry,
        };
        Ok(RegisteredMetadata(entry))
    }

    /// Schedule speculative candidate selection using an already-requested package version map.
    pub(crate) fn prefetch(
        &self,
        name: &PackageName,
        range: &Range<Version>,
        python_requirement: &PythonRequirement,
    ) -> Result<(), ResolveError> {
        if self.speculative.is_none() {
            self.sender.blocking_send(
                Request::Prefetch(name.clone(), range.clone(), python_requirement.clone()).into(),
            )?;
        }
        Ok(())
    }

    /// Acquire metadata registered during input preparation or package visitation.
    pub(crate) fn metadata(&self, dist: &Dist) -> Result<RegisteredMetadata<'_>, ResolveError> {
        if let Some(recorder) = &self.recorder {
            recorder.dependency_metadata(dist.name());
        }
        if let Some(speculative) = &self.speculative {
            return self
                .index
                .distributions()
                .get_registered(dist.distribution_id())
                .or_else(|| {
                    speculative
                        .distributions()
                        .get_registered(dist.distribution_id())
                })
                .map(RegisteredMetadata)
                .ok_or_else(|| ResolveError::UnregisteredTask(dist.to_string()));
        }
        self.index
            .distributions()
            .get_registered(dist.distribution_id())
            .map(RegisteredMetadata)
            .ok_or_else(|| ResolveError::UnregisteredTask(dist.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::thread;

    use tokio::sync::mpsc;
    use uv_distribution::ArchiveMetadata;
    use uv_distribution_filename::{DistExtension, SourceDistExtension};
    use uv_distribution_types::{Dist, Identifier, RequestedDist};
    use uv_normalize::PackageName;
    use uv_pep508::VerbatimUrl;
    use uv_pypi_types::ResolutionMetadata;

    use crate::ResolveError;
    use crate::resolver::{
        InMemoryIndex, MetadataResponse, MetadataUnavailable, Request, Response, VersionsResponse,
    };

    use super::{MetadataRequest, MetadataRequests, PendingVersions, RegisteredMetadata};

    fn wheel() -> Result<Dist, Box<dyn Error>> {
        let url = VerbatimUrl::parse_url("https://example.com/example-1.0.0-py3-none-any.whl")?;
        Ok(Dist::from_http_url(
            "example".parse()?,
            url.clone(),
            url.to_url(),
            None,
            DistExtension::Wheel,
        )?)
    }

    fn metadata() -> Result<MetadataResponse, Box<dyn Error>> {
        Ok(MetadataResponse::Found(ArchiveMetadata::from_metadata23(
            ResolutionMetadata::parse_metadata(
                b"Metadata-Version: 2.3\nName: example\nVersion: 1.0.0\n",
            )?,
        )))
    }

    #[test]
    fn speculative_package_error_allows_an_ordinary_retry() -> Result<(), Box<dyn Error>> {
        let index = InMemoryIndex::default();
        let (sender, mut receiver) = mpsc::channel(2);
        let requests = MetadataRequests::new(index.clone(), sender, None);
        let name: PackageName = "example".parse()?;

        let speculative = requests.speculative();
        let requested_name = name.clone();
        let worker = thread::spawn(move || {
            speculative
                .request_package(&requested_name, None)
                .map(PendingVersions::wait)
        });
        let request = receiver.blocking_recv().expect("optional request");
        assert!(matches!(request.request, Request::Package(..)));
        request
            .response
            .expect("optional response channel")
            .send(Err(ResolveError::ChannelClosed))
            .expect("optional caller is waiting");
        assert!(matches!(
            worker.join().expect("optional caller did not panic"),
            Err(ResolveError::ChannelClosed)
        ));
        assert!(index.implicit().get(&name).is_none());

        let pending = requests.request_package(&name, None)?;
        let request = receiver.try_recv().expect("ordinary retry was scheduled");
        assert!(matches!(request.request, Request::Package(..)));
        assert!(request.response.is_none());
        Response::Package(name.clone(), None, VersionsResponse::Found(Vec::new())).publish(&index);
        assert!(matches!(
            pending.wait().as_ref(),
            VersionsResponse::Found(_)
        ));
        Ok(())
    }

    #[test]
    fn speculative_metadata_error_allows_an_ordinary_retry() -> Result<(), Box<dyn Error>> {
        let index = InMemoryIndex::default();
        let (sender, mut receiver) = mpsc::channel(2);
        let requests = MetadataRequests::new(index.clone(), sender, None);
        let dist = wheel()?;
        let id = dist.distribution_id();

        let speculative = requests.speculative();
        let requested_dist = dist.clone();
        let worker = thread::spawn(move || {
            speculative
                .request_metadata(MetadataRequest::Dist(requested_dist), |_| Ok(()))
                .map(|registered| registered.wait())
        });
        let request = receiver.blocking_recv().expect("optional request");
        assert!(matches!(request.request, Request::Dist(_)));
        request
            .response
            .expect("optional response channel")
            .send(Ok(Some(Response::Dist {
                dist: dist.clone(),
                metadata: MetadataResponse::Error(
                    Box::new(RequestedDist::Installable(dist.clone())),
                    Arc::new(uv_distribution::Error::NoBuild),
                ),
            })))
            .expect("optional caller is waiting");
        assert!(matches!(
            worker.join().expect("optional caller did not panic"),
            Err(ResolveError::Dist(..))
        ));
        assert!(index.distributions().get(&id).is_none());

        let pending = requests.request_metadata(MetadataRequest::Dist(dist.clone()), |_| Ok(()))?;
        let request = receiver.try_recv().expect("ordinary retry was scheduled");
        assert!(matches!(request.request, Request::Dist(_)));
        assert!(request.response.is_none());
        Response::Dist {
            dist,
            metadata: metadata()?,
        }
        .publish(&index);
        assert!(matches!(
            pending.wait().as_ref(),
            MetadataResponse::Found(_)
        ));
        Ok(())
    }

    #[test]
    fn speculative_negative_versions_stay_in_the_trial() -> Result<(), Box<dyn Error>> {
        for response in [
            VersionsResponse::NotFound,
            VersionsResponse::NoIndex,
            VersionsResponse::Offline,
        ] {
            let expected = std::mem::discriminant(&response);
            let index = InMemoryIndex::default();
            let (sender, mut receiver) = mpsc::channel(2);
            let requests = MetadataRequests::new(index.clone(), sender, None);
            let name: PackageName = "example".parse()?;
            let trial = requests.speculative();
            let speculative = trial.clone();
            let requested_name = name.clone();
            let worker = thread::spawn(move || {
                speculative
                    .request_package(&requested_name, None)
                    .map(PendingVersions::wait)
            });
            receiver
                .blocking_recv()
                .expect("optional request")
                .response
                .expect("optional response channel")
                .send(Ok(Some(Response::Package(name.clone(), None, response))))
                .expect("optional caller is waiting");
            worker.join().expect("optional caller did not panic")?;
            assert!(index.implicit().get(&name).is_none());
            assert_eq!(
                std::mem::discriminant(trial.request_package(&name, None)?.wait().as_ref()),
                expected
            );

            let pending = requests.request_package(&name, None)?;
            let request = receiver.try_recv().expect("ordinary retry was scheduled");
            assert!(request.response.is_none());
            Response::Package(name.clone(), None, VersionsResponse::Found(Vec::new()))
                .publish(&index);
            assert!(matches!(
                pending.wait().as_ref(),
                VersionsResponse::Found(_)
            ));
            // An ordinary result supersedes an earlier negative observation in this trial.
            assert!(matches!(
                trial.request_package(&name, None)?.wait().as_ref(),
                VersionsResponse::Found(_)
            ));
        }
        Ok(())
    }

    #[test]
    fn speculative_unavailable_metadata_stays_in_the_trial() -> Result<(), Box<dyn Error>> {
        let index = InMemoryIndex::default();
        let (sender, mut receiver) = mpsc::channel(2);
        let requests = MetadataRequests::new(index.clone(), sender, None);
        let dist = wheel()?;
        let id = dist.distribution_id();
        let trial = requests.speculative();
        let speculative = trial.clone();
        let requested_dist = dist.clone();
        let worker = thread::spawn(move || {
            speculative
                .request_metadata(MetadataRequest::Dist(requested_dist), |_| Ok(()))
                .map(RegisteredMetadata::wait)
        });
        receiver
            .blocking_recv()
            .expect("optional request")
            .response
            .expect("optional response channel")
            .send(Ok(Some(Response::Dist {
                dist: dist.clone(),
                metadata: MetadataResponse::Unavailable(MetadataUnavailable::Network(
                    reqwest::StatusCode::SERVICE_UNAVAILABLE,
                )),
            })))
            .expect("optional caller is waiting");
        worker.join().expect("optional caller did not panic")?;
        assert!(index.distributions().get(&id).is_none());
        assert!(matches!(
            trial.metadata(&dist)?.wait().as_ref(),
            MetadataResponse::Unavailable(MetadataUnavailable::Network(_))
        ));

        let pending = requests.request_metadata(MetadataRequest::Dist(dist.clone()), |_| Ok(()))?;
        let request = receiver.try_recv().expect("ordinary retry was scheduled");
        assert!(request.response.is_none());
        Response::Dist {
            dist,
            metadata: metadata()?,
        }
        .publish(&index);
        assert!(matches!(
            pending.wait().as_ref(),
            MetadataResponse::Found(_)
        ));
        Ok(())
    }

    #[test]
    fn speculative_source_build_is_not_started() -> Result<(), Box<dyn Error>> {
        let index = InMemoryIndex::default();
        let (sender, mut receiver) = mpsc::channel(2);
        let requests = MetadataRequests::new(index.clone(), sender, None);
        let url = VerbatimUrl::parse_url("https://example.com/example-1.0.0.tar.gz")?;
        let dist = Dist::from_http_url(
            "example".parse()?,
            url.clone(),
            url.to_url(),
            None,
            DistExtension::Source(SourceDistExtension::TarGz),
        )?;
        let id = dist.distribution_id();
        let trial = requests.speculative();
        assert!(matches!(
            trial.request_metadata(MetadataRequest::Dist(dist.clone()), |_| Ok(())),
            Err(ResolveError::Distribution(uv_distribution::Error::NoBuild))
        ));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        assert!(index.distributions().get(&id).is_none());

        let pending = requests.request_metadata(MetadataRequest::Dist(dist.clone()), |_| Ok(()))?;
        let request = receiver.try_recv().expect("ordinary source request");
        assert!(matches!(request.request, Request::Dist(Dist::Source(_))));
        assert!(request.response.is_none());
        Response::Dist {
            dist: dist.clone(),
            metadata: metadata()?,
        }
        .publish(&index);
        pending.wait();
        trial.request_metadata(MetadataRequest::Dist(dist), |_| {
            Err(ResolveError::ChannelClosed)
        })?;
        Ok(())
    }
}
