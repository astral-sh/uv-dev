use std::collections::hash_map::Entry;

use rustc_hash::FxHashMap;

use uv_distribution_types::{DistributionId, Identifier, ResolvedDist};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_resolver_types::PreferenceId;

use crate::ResolveError;
use crate::candidate_selector::Candidate;
use crate::prioritized_distribution::CompatibleDist;
use crate::resolver::RegisteredMetadata;

#[derive(Clone, Debug)]
enum FilePin<'index> {
    Registry {
        /// The concrete distribution chosen for installation and locking.
        dist: ResolvedDist,
        /// The concrete distribution whose metadata is used during resolution.
        metadata: PinMetadata<'index>,
        /// Input preferences used to select this package, including selections through proxies.
        preferences: Vec<PreferenceId>,
    },
    Url(RegisteredMetadata<'index>),
}

#[derive(Clone, Debug)]
enum PinMetadata<'index> {
    /// Proxy selection and direct-only resolution do not require a metadata request.
    Unrequested(DistributionId),
    Registered(RegisteredMetadata<'index>),
}

/// The artifacts and metadata selected for package versions within a fork.
#[derive(Clone, Debug, Default)]
pub(crate) struct FilePins<'index>(FxHashMap<(PackageName, Version), FilePin<'index>>);

impl<'index> FilePins<'index> {
    /// Pin a registry candidate, registering its metadata at most once in this fork.
    ///
    /// Within a fork, each `(name, version)` selects the same artifact. Proxy packages may pin it
    /// before its real package is selected; upgrade that pin when metadata is requested.
    pub(crate) fn insert(
        &mut self,
        candidate: &Candidate,
        dist: &CompatibleDist,
        request: Option<impl FnOnce() -> Result<RegisteredMetadata<'index>, ResolveError>>,
    ) -> Result<(), ResolveError> {
        match self
            .0
            .entry((candidate.name().clone(), candidate.version().clone()))
        {
            Entry::Occupied(mut entry) => {
                if let Some(preference_id) = candidate.preference_id()
                    && let FilePin::Registry { preferences, .. } = entry.get_mut()
                    && !preferences.contains(&preference_id)
                {
                    preferences.push(preference_id);
                }
                if let Some(request) = request
                    && let FilePin::Registry {
                        metadata: metadata @ PinMetadata::Unrequested(_),
                        ..
                    } = entry.get_mut()
                {
                    *metadata = PinMetadata::Registered(request()?);
                }
            }
            Entry::Vacant(entry) => {
                let metadata = if let Some(request) = request {
                    PinMetadata::Registered(request()?)
                } else {
                    PinMetadata::Unrequested(dist.for_resolution().distribution_id())
                };
                entry.insert(FilePin::Registry {
                    dist: dist.for_installation().to_owned(),
                    metadata,
                    preferences: candidate.preference_id().into_iter().collect(),
                });
            }
        }
        Ok(())
    }

    /// Retain the metadata used to select a URL package's version.
    pub(crate) fn insert_url(
        &mut self,
        name: &PackageName,
        version: &Version,
        metadata: RegisteredMetadata<'index>,
    ) {
        self.0
            .entry((name.clone(), version.clone()))
            .or_insert(FilePin::Url(metadata));
    }

    /// Return the pinned registry artifact, if one exists.
    pub(crate) fn get(&self, name: &PackageName, version: &Version) -> Option<&ResolvedDist> {
        match self.0.get(&(name.clone(), version.clone()))? {
            FilePin::Registry { dist, .. } => Some(dist),
            FilePin::Url(_) => None,
        }
    }

    /// Return the metadata registered when selecting this package version.
    pub(crate) fn metadata(
        &self,
        name: &PackageName,
        version: &Version,
    ) -> Option<&RegisteredMetadata<'index>> {
        match self.0.get(&(name.clone(), version.clone()))? {
            FilePin::Registry {
                metadata: PinMetadata::Registered(metadata),
                ..
            }
            | FilePin::Url(metadata) => Some(metadata),
            FilePin::Registry {
                metadata: PinMetadata::Unrequested(_),
                ..
            } => None,
        }
    }

    /// Return the registry artifact, metadata identity, and input preferences in a single lookup.
    pub(crate) fn dist_and_id(
        &self,
        name: &PackageName,
        version: &Version,
    ) -> Option<(&ResolvedDist, &DistributionId, &[PreferenceId])> {
        match self.0.get(&(name.clone(), version.clone()))? {
            FilePin::Registry {
                dist,
                metadata,
                preferences,
            } => Some((
                dist,
                match metadata {
                    PinMetadata::Unrequested(id) => id,
                    PinMetadata::Registered(metadata) => metadata.id(),
                },
                preferences,
            )),
            FilePin::Url(_) => None,
        }
    }
}
