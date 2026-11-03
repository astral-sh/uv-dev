//! Freshness of the mutable declarations used to resolve isolated build environments.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::path::Path;

use serde::{Deserialize, Serialize};
use uv_distribution_types::{
    ExtraBuildRequires, IndexUrl, Requirement, RequirementSource, RequiresPython,
};
use uv_fs::{PortablePath, try_relative_to_if};
use uv_normalize::PackageName;
use uv_pypi_types::HashDigests;
use uv_resolver_types::UniversalMarker;

use super::{Lock, LockError, LockErrorKind, Source, absolute_path, normalize_requirement};

/// A declaration that can change independently of an immutable distribution.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) struct BuildInput {
    kind: BuildInputKind,
    #[serde(flatten)]
    requirement: Requirement,
    /// Source normalization discards URL fragments. Retain the declared artifact digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
    /// A file index is relative to the lock like other local sources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_index: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum BuildInputKind {
    Backend,
    Extra,
    MatchRuntime,
}

impl BuildInput {
    fn normalize(
        mut self,
        root: &Path,
        python: &RequiresPython,
        domain: UniversalMarker,
    ) -> Result<Option<Self>, LockError> {
        if let Some(hashes) = self
            .requirement
            .hashes()
            .map_err(|error| LockErrorKind::BuildGraph(error.to_string()))?
        {
            self.hash = HashDigests::from(hashes).first().map(ToString::to_string);
        }
        let mut requirement = normalize_requirement(self.requirement, root, python)?;
        // Build roots have no active extras. Retain all applicable platform and Python forks,
        // independent of the interpreter performing the check.
        requirement.marker = requirement
            .marker
            .resolve_extras([])
            .restrict(python.simplify_markers(domain.combined()));
        if requirement.marker.is_false() {
            return Ok(None);
        }
        if let RequirementSource::Registry { index, .. } = &mut requirement.source
            && let Some(metadata) = index
            && let IndexUrl::Path(url) = &metadata.url
        {
            let path = url.to_file_path().map_err(|()| LockErrorKind::UrlToPath {
                url: url.to_url().into_url(),
            })?;
            let path =
                try_relative_to_if(path, root, true).map_err(LockErrorKind::IndexRelativePath)?;
            self.local_index = Some(PortablePath::from(path.as_path()).to_string());
            *index = None;
        }
        requirement.set_force_relative(true);
        self.requirement = requirement
            .relative_to(root)
            .map_err(LockErrorKind::RequirementRelativePath)?;
        Ok(Some(self))
    }

    pub(super) fn collect(
        backend: &[Requirement],
        extras: &ExtraBuildRequires,
        owner: &PackageName,
        root: &Path,
        python: &RequiresPython,
        domain: UniversalMarker,
    ) -> Result<BTreeSet<Self>, LockError> {
        let backend = backend.iter().map(|req| (BuildInputKind::Backend, req));
        let extras = extras.get(owner).into_iter().flatten().map(|extra| {
            let kind = if extra.match_runtime {
                BuildInputKind::MatchRuntime
            } else {
                BuildInputKind::Extra
            };
            (kind, &extra.requirement)
        });
        backend
            .chain(extras)
            .filter_map(|(kind, requirement)| {
                Self {
                    kind,
                    hash: None,
                    requirement: requirement.clone(),
                    local_index: None,
                }
                .normalize(root, python, domain)
                .transpose()
            })
            .collect()
    }
}

impl Source {
    pub(super) fn mutable_build_directory(&self) -> Option<&Path> {
        match self {
            Self::Directory(path) | Self::Editable(path) => Some(path),
            Self::Registry(_)
            | Self::Git(_, _)
            | Self::Direct(_, _)
            | Self::Path(_)
            | Self::Virtual(_) => None,
        }
    }
}

impl Lock {
    /// Check the declarations used by each build without inspecting immutable source distributions.
    pub async fn satisfies_build_inputs<E: From<LockError>>(
        &self,
        root: &Path,
        extras: &ExtraBuildRequires,
        read_directory: impl AsyncFn(&PackageName, &Path) -> Result<Vec<Requirement>, E>,
    ) -> Result<bool, E> {
        let project = self.build_project_domain();
        let mut directories = BTreeMap::new();
        for universe in self.builds.iter().flat_map(|builds| &builds.universes) {
            let Some(inputs) = &universe.inputs else {
                // Older preview locks lack the declarations needed for a checked replay.
                return Ok(false);
            };
            let backend = if let Some(path) = universe.owner.source.mutable_build_directory() {
                match directories.entry((&universe.owner.name, path)) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => entry.insert(
                        read_directory(&universe.owner.name, &absolute_path(root, path)?).await?,
                    ),
                }
                .as_slice()
            } else {
                &[]
            };
            let domain = UniversalMarker::from_combined(universe.marker.combined().and(project));
            let expected = BuildInput::collect(
                backend,
                extras,
                &universe.owner.name,
                root,
                &self.requires_python,
                domain,
            )?;
            // Wire paths are relative to the lock. Reconstruct both sides against the same root.
            let locked = inputs
                .iter()
                .cloned()
                .filter_map(|input| {
                    input
                        .normalize(root, &self.requires_python, domain)
                        .transpose()
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            if expected != locked {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
