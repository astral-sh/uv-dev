use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use tokio::sync::mpsc::Sender;

use uv_distribution_types::{
    Dist, DistributionId, Identifier, IndexMetadata, IndexUrl, Name, ResolutionRecorder,
};
use uv_normalize::PackageName;
use uv_once_map::Registration;
use uv_pep440::Version;

use crate::prioritized_distribution::RegistryVariantsJson;
use crate::pubgrub::Range;
use crate::resolved::ResolvedDistRef;
use crate::resolver::index::{FxRegisteredEntry, ParsedVariantsJsonResponse};
use crate::resolver::{
    InMemoryIndex, MetadataResponse, MetadataTarget, PackageProducer, Request, VersionsResponse,
};
use crate::{PythonRequirement, ResolveError};

/// A blocking solver-side handle for requesting metadata from the asynchronous fetcher.
#[derive(Clone)]
pub(crate) struct MetadataRequests {
    index: InMemoryIndex,
    sender: Sender<Request>,
    recorder: Option<ResolutionRecorder>,
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

    fn into_target(self) -> MetadataTarget {
        match self {
            Self::Dist(dist) => MetadataTarget::Dist(Arc::new(dist)),
            Self::Resolved(dist) => MetadataTarget::from(dist),
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
enum PendingVersionsEntry<'index> {
    Implicit(FxRegisteredEntry<'index, PackageName, Arc<VersionsResponse>>),
    Explicit(FxRegisteredEntry<'index, (PackageName, IndexUrl), Arc<VersionsResponse>>),
}

impl PendingVersionsEntry<'_> {
    fn wait(&self) -> Option<Arc<VersionsResponse>> {
        match self {
            Self::Implicit(entry) => entry.wait_blocking(),
            Self::Explicit(entry) => entry.wait_blocking(),
        }
    }
}

pub(crate) struct PendingVersions<'index> {
    requests: &'index MetadataRequests,
    name: PackageName,
    index: Option<IndexMetadata>,
    entry: PendingVersionsEntry<'index>,
}

impl PendingVersions<'_> {
    pub(crate) fn wait(mut self) -> Result<Arc<VersionsResponse>, ResolveError> {
        loop {
            if let Some(versions) = self.entry.wait() {
                return Ok(versions);
            }
            self.entry = self
                .requests
                .register_package(&self.name, self.index.as_ref())?;
        }
    }
}

/// A distribution whose metadata was registered or supplied before resolution.
///
/// Selected pins retain a registered entry that prevents explicit cache removal while borrowed.
#[derive(Clone)]
pub(crate) struct RegisteredMetadata<'index> {
    requests: &'index MetadataRequests,
    target: MetadataTarget,
    entry: FxRegisteredEntry<'index, DistributionId, Arc<MetadataResponse>>,
}

impl Debug for RegisteredMetadata<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisteredMetadata")
            .field("target", &self.target)
            .field("entry", &self.entry)
            .finish()
    }
}

impl RegisteredMetadata<'_> {
    pub(crate) fn id(&self) -> &DistributionId {
        self.entry.key()
    }

    pub(crate) fn wait(&self) -> Result<Arc<MetadataResponse>, ResolveError> {
        let mut entry = self.entry.clone();
        loop {
            if let Some(metadata) = entry.wait_blocking() {
                return Ok(metadata);
            }
            entry = self.requests.register_metadata(&self.target)?;
        }
    }
}

/// Index-level variant metadata registered for a selected package version.
#[derive(Clone)]
pub(crate) struct RegisteredVariantsJson<'index> {
    requests: &'index MetadataRequests,
    entry: FxRegisteredEntry<'index, RegistryVariantsJson, Arc<ParsedVariantsJsonResponse>>,
}

impl Debug for RegisteredVariantsJson<'_> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegisteredVariantsJson")
            .field("entry", &self.entry)
            .finish()
    }
}

impl RegisteredVariantsJson<'_> {
    pub(crate) fn wait(&self) -> Result<Arc<ParsedVariantsJsonResponse>, ResolveError> {
        let mut entry = self.entry.clone();
        loop {
            if let Some(response) = entry.wait_blocking() {
                return Ok(response);
            }
            let target = entry.key().clone();
            entry = self.requests.register_variants_json(&target)?;
        }
    }
}

impl MetadataRequests {
    pub(crate) fn new(
        index: InMemoryIndex,
        sender: Sender<Request>,
        recorder: Option<ResolutionRecorder>,
    ) -> Self {
        Self {
            index,
            sender,
            recorder,
        }
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
        self.register_package(name, index)?;
        Ok(())
    }

    fn register_package<'index>(
        &'index self,
        name: &PackageName,
        index: Option<&IndexMetadata>,
    ) -> Result<PendingVersionsEntry<'index>, ResolveError> {
        if let Some(index) = index {
            match self
                .index
                .explicit()
                .register_entry_with_producer((name.clone(), index.url().clone()))
            {
                Registration::New((entry, producer)) => {
                    self.sender.blocking_send(Request::Package {
                        package_name: name.clone(),
                        index: Some(index.clone()),
                        producer: PackageProducer::Explicit(producer),
                    })?;
                    Ok(PendingVersionsEntry::Explicit(entry))
                }
                Registration::Existing(entry) => Ok(PendingVersionsEntry::Explicit(entry)),
            }
        } else {
            match self
                .index
                .implicit()
                .register_entry_with_producer(name.clone())
            {
                Registration::New((entry, producer)) => {
                    self.sender.blocking_send(Request::Package {
                        package_name: name.clone(),
                        index: None,
                        producer: PackageProducer::Implicit(producer),
                    })?;
                    Ok(PendingVersionsEntry::Implicit(entry))
                }
                Registration::Existing(entry) => Ok(PendingVersionsEntry::Implicit(entry)),
            }
        }
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
        let entry = self.register_package(name, index)?;
        Ok(PendingVersions {
            requests: self,
            name: name.clone(),
            index: index.cloned(),
            entry,
        })
    }

    /// Schedule metadata retrieval without retaining or cloning its cache identity.
    pub(crate) fn enqueue_metadata(
        &self,
        request: MetadataRequest<'_>,
    ) -> Result<(), ResolveError> {
        if let Some(recorder) = &self.recorder {
            recorder.dependency_metadata(request.name());
        }
        if let Some(producer) = self
            .index
            .distributions()
            .register_with_producer(request.id())
        {
            self.sender
                .blocking_send(request.into_target().into_request(producer))?;
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
        let (entry, target) = match self
            .index
            .distributions()
            .register_entry_with_producer(request.id())
        {
            Registration::New((entry, producer)) => {
                validate(&request)?;
                let target = request.into_target();
                self.sender
                    .blocking_send(target.clone().into_request(producer))?;
                (entry, target)
            }
            Registration::Existing(entry) => (entry, request.into_target()),
        };
        Ok(RegisteredMetadata {
            requests: self,
            target,
            entry,
        })
    }

    fn register_metadata<'index>(
        &'index self,
        target: &MetadataTarget,
    ) -> Result<FxRegisteredEntry<'index, DistributionId, Arc<MetadataResponse>>, ResolveError>
    {
        match self
            .index
            .distributions()
            .register_entry_with_producer(target.id())
        {
            Registration::New((entry, producer)) => {
                self.sender
                    .blocking_send(target.clone().into_request(producer))?;
                Ok(entry)
            }
            Registration::Existing(entry) => Ok(entry),
        }
    }

    /// Schedule variant metadata retrieval once for the selected package version.
    pub(crate) fn request_variants_json(
        &self,
        variants_json: &RegistryVariantsJson,
    ) -> Result<RegisteredVariantsJson<'_>, ResolveError> {
        let entry = self.register_variants_json(variants_json)?;
        Ok(RegisteredVariantsJson {
            requests: self,
            entry,
        })
    }

    fn register_variants_json<'index>(
        &'index self,
        variants_json: &RegistryVariantsJson,
    ) -> Result<
        FxRegisteredEntry<'index, RegistryVariantsJson, Arc<ParsedVariantsJsonResponse>>,
        ResolveError,
    > {
        match self
            .index
            .variants_json()
            .register_entry_with_producer(variants_json.clone())
        {
            Registration::New((entry, producer)) => {
                self.sender.blocking_send(Request::VariantsJson {
                    variants_json: variants_json.clone(),
                    producer,
                })?;
                Ok(entry)
            }
            Registration::Existing(entry) => Ok(entry),
        }
    }

    /// Schedule speculative candidate selection using an already-requested package version map.
    pub(crate) fn prefetch(
        &self,
        name: &PackageName,
        range: &Range<Version>,
        python_requirement: &PythonRequirement,
    ) -> Result<(), ResolveError> {
        self.sender.blocking_send(Request::Prefetch(
            name.clone(),
            range.clone(),
            python_requirement.clone(),
        ))?;
        Ok(())
    }

    /// Acquire metadata registered during input preparation or package visitation.
    pub(crate) fn metadata(&self, dist: &Dist) -> Result<RegisteredMetadata<'_>, ResolveError> {
        if let Some(recorder) = &self.recorder {
            recorder.dependency_metadata(dist.name());
        }
        let target = MetadataTarget::Dist(Arc::new(dist.clone()));
        let entry = self
            .index
            .distributions()
            .get_registered(target.id())
            .ok_or_else(|| ResolveError::UnregisteredTask(dist.to_string()))?;
        Ok(RegisteredMetadata {
            requests: self,
            target,
            entry,
        })
    }
}
