use hashbrown::Equivalent;
use hashbrown::hash_map::EntryRef;

use uv_distribution_types::{DistributionId, Identifier, ResolvedDist};
use uv_normalize::PackageName;
use uv_pep440::Version;

use crate::candidate_selector::Candidate;
use crate::prioritized_distribution::CompatibleDist;
use crate::resolver::RegisteredMetadata;
use crate::{FxHashbrownMap, ResolveError};

#[derive(Clone, Debug)]
enum FilePin<'index> {
    Registry {
        /// The concrete distribution chosen for installation and locking.
        dist: ResolvedDist,
        /// The concrete distribution whose metadata is used during resolution.
        metadata: PinMetadata<'index>,
    },
    Url(RegisteredMetadata<'index>),
}

#[derive(Clone, Debug)]
enum PinMetadata<'index> {
    /// Proxy selection and direct-only resolution do not require a metadata request.
    Unrequested(DistributionId),
    Registered(RegisteredMetadata<'index>),
}

/// A borrowed pin key. Wrapping the tuple makes its derived hash match the stored tuple's hash.
#[derive(Hash)]
struct PinKeyRef<'a>((&'a PackageName, &'a Version));

impl PinKeyRef<'_> {
    fn to_owned(&self) -> (PackageName, Version) {
        let (name, version) = self.0;
        (name.clone(), version.clone())
    }
}

impl Equivalent<(PackageName, Version)> for PinKeyRef<'_> {
    fn equivalent(&self, (name, version): &(PackageName, Version)) -> bool {
        self.0 == (name, version)
    }
}

/// The artifacts and metadata selected for package versions within a fork.
#[derive(Clone, Debug, Default)]
pub(crate) struct FilePins<'index>(FxHashbrownMap<(PackageName, Version), FilePin<'index>>);

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
        let key = PinKeyRef((candidate.name(), candidate.version()));
        match self.0.entry_ref(&key) {
            EntryRef::Occupied(mut entry) => {
                if let Some(request) = request
                    && let FilePin::Registry {
                        metadata: metadata @ PinMetadata::Unrequested(_),
                        ..
                    } = entry.get_mut()
                {
                    *metadata = PinMetadata::Registered(request()?);
                }
            }
            EntryRef::Vacant(entry) => {
                let metadata = if let Some(request) = request {
                    PinMetadata::Registered(request()?)
                } else {
                    PinMetadata::Unrequested(dist.for_resolution().distribution_id())
                };
                entry.insert_with_key(
                    key.to_owned(),
                    FilePin::Registry {
                        dist: dist.for_installation().to_owned(),
                        metadata,
                    },
                );
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
        let key = PinKeyRef((name, version));
        if let EntryRef::Vacant(entry) = self.0.entry_ref(&key) {
            entry.insert_with_key(key.to_owned(), FilePin::Url(metadata));
        }
    }

    /// Return the pinned registry artifact, if one exists.
    pub(crate) fn get(&self, name: &PackageName, version: &Version) -> Option<&ResolvedDist> {
        match self.0.get(&PinKeyRef((name, version)))? {
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
        match self.0.get(&PinKeyRef((name, version)))? {
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

    /// Return the pinned registry artifact and its metadata identity in a single lookup.
    pub(crate) fn dist_and_id(
        &self,
        name: &PackageName,
        version: &Version,
    ) -> Option<(&ResolvedDist, &DistributionId)> {
        match self.0.get(&PinKeyRef((name, version)))? {
            FilePin::Registry { dist, metadata } => Some((
                dist,
                match metadata {
                    PinMetadata::Unrequested(id) => id,
                    PinMetadata::Registered(metadata) => metadata.id(),
                },
            )),
            FilePin::Url(_) => None,
        }
    }
}
