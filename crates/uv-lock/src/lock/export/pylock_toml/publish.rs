use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use jiff::Timestamp;

use uv_configuration::{DependencyGroupsWithDefaults, ExtrasSpecification, InstallOptions};
use uv_distribution::FlatRequiresDist;
use uv_distribution_filename::DistFilename;
use uv_distribution_types::{
    File, IndexUrl, PYPI_URL, RemoteSource, Requirement, RequirementSource, RequiresPython,
};
use uv_normalize::{DefaultExtras, ExtraName, PackageName};
use uv_pep440::Version;
use uv_pep508::MarkerTree;
use uv_pypi_types::{Hashes, ResolutionMetadata};
use uv_redacted::SafeUrl;
use uv_redacted::policy::Raw;

use crate::lock::export::{ExportableRequirements, VirtualPackages};
use crate::lock::requirements::RequirementNormalizer;
use crate::lock::{Source, SourceDist};
use crate::{
    Installable, Lock, LockError, Package, PylockToml, PylockTomlError, PylockTomlErrorKind,
    implicit_constraints_marker,
};

use super::{PylockTomlSdist, PylockTomlWheel};

#[derive(Debug, thiserror::Error)]
pub enum PublishLockError {
    #[error("Cannot export `{0}` from `uv.lock`: it depends on non-registry package `{1}`")]
    NonRegistry(PackageName, PackageName),
    #[error(
        "Cannot export `{package}` from `uv.lock`: dependency `{dependency}` uses the non-PyPI index `{index}`; pass `--allow-non-pypi` to export it"
    )]
    NonPypi {
        package: PackageName,
        dependency: PackageName,
        index: SafeUrl,
    },
    #[error("Cannot export the non-PyPI index `{0}`; pass `--allow-non-pypi` to export it")]
    NonPypiIndex(SafeUrl),
    #[error("Cannot export an unsupported index URL: `{0}`")]
    InvalidIndex(SafeUrl),
    #[error("Cannot export `{0}` from `uv.lock`: workspace dependency `{1}` has no locked version")]
    MissingWorkspaceVersion(PackageName, PackageName),
    #[error("Cannot export `{0}` from `uv.lock`: dependency `{1}` has no registry artifacts")]
    MissingArtifacts(PackageName, PackageName),
    #[error("Cannot export `{0}` from `uv.lock`: the resolution does not satisfy `{1}`")]
    UnsatisfiedRequirement(PackageName, Box<Requirement>),
    #[error("Cannot export a lock with missing artifact hashes; regenerate `uv.lock`")]
    MissingHashes,
    #[error("Cannot export artifact `{filename}` for `{package}` with an unsupported URL: `{url}`")]
    ArtifactUrl {
        package: PackageName,
        filename: String,
        url: SafeUrl,
    },
    #[error("Cannot export artifact `{0}`: expected `{1}=={2}`")]
    ArtifactIdentity(String, PackageName, Version),
    #[error("Cannot export multiple source distributions for `{0}`")]
    MultipleSourceDistributions(PackageName),
    #[error("Cannot rewrite workspace dependency `{0}`: it is missing from the export")]
    MissingWorkspaceDependency(PackageName),
    #[error(transparent)]
    Pylock(#[from] PylockTomlError),
}

impl PylockToml {
    /// Prepare a runtime-only export and identify its reachable workspace dependencies.
    ///
    /// The caller must replace each workspace dependency with its uploaded registry artifacts
    /// before serializing the lock.
    pub fn from_publish_lock(
        lock: &Lock,
        root: &Path,
        project: &ResolutionMetadata,
        allow_non_pypi: bool,
    ) -> Result<(Self, BTreeMap<PackageName, Version>), PublishLockError> {
        let name = &project.name;
        let target = PublishTarget { root, lock, name };
        let extras = ExtrasSpecification::default().with_defaults(DefaultExtras::default());
        let groups = DependencyGroupsWithDefaults::none();
        let install_options =
            InstallOptions::new(true, false, false, false, false, false, vec![], vec![]);
        let ExportableRequirements(nodes) = ExportableRequirements::from_lock(
            &target,
            &[],
            &extras,
            &groups,
            false,
            &install_options,
            VirtualPackages::Include,
        )
        .map_err(PylockTomlError::from)?;

        let mut workspace = BTreeMap::new();
        for node in &nodes {
            let package = node.package;
            if let Some(index) = package.index(root).map_err(PylockTomlError::from)? {
                if !allow_non_pypi && !is_pypi(index.url()) {
                    return Err(PublishLockError::NonPypi {
                        package: name.clone(),
                        dependency: package.name().clone(),
                        index: index.url().clone(),
                    });
                }
                if package.sdist.is_none() && package.wheels.is_empty() {
                    return Err(PublishLockError::MissingArtifacts(
                        name.clone(),
                        package.name().clone(),
                    ));
                }
                if package
                    .sdist
                    .as_ref()
                    .is_some_and(|sdist| !matches!(sdist, SourceDist::Url { .. }))
                {
                    return Err(PublishLockError::MissingArtifacts(
                        name.clone(),
                        package.name().clone(),
                    ));
                }
                if package
                    .wheels
                    .iter()
                    .any(|wheel| wheel.filename.variant().is_some())
                {
                    return Err(PylockTomlError::from(
                        PylockTomlErrorKind::WheelVariantUnsupported(package.name().clone()),
                    )
                    .into());
                }
            } else if lock.is_workspace_member(package)
                && matches!(
                    package.id.source,
                    Source::Directory(_) | Source::Editable(_)
                )
            {
                let version = package.version().cloned().ok_or_else(|| {
                    PublishLockError::MissingWorkspaceVersion(name.clone(), package.name().clone())
                })?;
                workspace.insert(package.name().clone(), version);
            } else {
                return Err(PublishLockError::NonRegistry(
                    name.clone(),
                    package.name().clone(),
                ));
            }
        }

        // A project resolution can apply overrides and exclusions. The artifact retains the
        // published requirements, so every active declaration still needs a matching lock entry.
        let supported = implicit_constraints_marker(
            lock.requires_python().to_marker_tree(),
            lock.supported_environments(),
        );
        let requirements = FlatRequiresDist::from_requirements(
            project
                .requires_dist
                .iter()
                .cloned()
                .map(Requirement::from)
                .collect(),
            name,
        );
        for requirement in requirements {
            let required = supported.and(requirement.marker);
            if !required.evaluate_optional_environment(None, &[]) {
                continue;
            }
            let covered = match &requirement.source {
                RequirementSource::Registry { specifier, .. } if &requirement.name == name => {
                    if specifier.contains(&project.version) {
                        MarkerTree::TRUE
                    } else {
                        MarkerTree::FALSE
                    }
                }
                RequirementSource::Registry { specifier, .. } => nodes
                    .iter()
                    .filter(|node| {
                        node.package.name() == &requirement.name
                            && node
                                .package
                                .version()
                                .is_some_and(|version| specifier.contains(version))
                    })
                    .fold(MarkerTree::FALSE, |covered, node| covered.or(node.marker)),
                RequirementSource::Url { .. }
                | RequirementSource::GitDirectory { .. }
                | RequirementSource::GitPath { .. }
                | RequirementSource::Path { .. }
                | RequirementSource::Directory { .. } => MarkerTree::FALSE,
            };
            if required
                .and(covered.negate())
                .evaluate_optional_environment(None, &[])
            {
                return Err(PublishLockError::UnsatisfiedRequirement(
                    name.clone(),
                    Box::new(requirement),
                ));
            }
        }

        let pylock = Self::from_lock(
            &target,
            root,
            &[],
            &extras,
            &groups,
            false,
            None,
            &install_options,
        )
        .map_err(PylockTomlError::from)?;

        for package in &pylock.packages {
            if let Some(index) = &package.index {
                check_url(&package.name, "index", index.as_url(), allow_non_pypi)?;
            }
            if let Some(sdist) = &package.sdist
                && let Some(url) = &sdist.url
            {
                check_url(
                    &package.name,
                    "source distribution",
                    url.as_url(),
                    allow_non_pypi,
                )?;
            }
            for wheel in package.wheels.iter().flatten() {
                if let Some(url) = &wheel.url {
                    check_url(&package.name, "wheel", url.as_url(), allow_non_pypi)?;
                }
            }
        }
        if pylock.has_missing_hashes() {
            return Err(PublishLockError::MissingHashes);
        }

        Ok((pylock, workspace))
    }

    /// Validate the destination index before exporting any workspace artifacts.
    pub fn check_publish_index(
        index: &IndexUrl,
        allow_non_pypi: bool,
    ) -> Result<(), PublishLockError> {
        if !allow_non_pypi && !is_pypi(index.url()) {
            return Err(PublishLockError::NonPypiIndex(index.url().clone()));
        }
        if !is_publish_url(index.url(), allow_non_pypi) {
            return Err(PublishLockError::InvalidIndex(index.url().clone()));
        }
        Ok(())
    }

    /// Replace a workspace entry with the registry files from this publishing session.
    pub fn replace_publish_dependency(
        &mut self,
        name: &PackageName,
        version: &Version,
        index: &IndexUrl,
        files: &[(DistFilename, File)],
        allow_non_pypi: bool,
    ) -> Result<(), PublishLockError> {
        Self::check_publish_index(index, allow_non_pypi)?;

        let mut sdist = None;
        let mut wheels = Vec::new();
        for (filename, file) in files {
            if filename.name() != name || filename.version() != version {
                return Err(PublishLockError::ArtifactIdentity(
                    file.filename.to_string(),
                    name.clone(),
                    version.clone(),
                ));
            }
            let url = file.url.to_url().map_err(PylockTomlError::from)?;
            check_url(name, &file.filename, &url, allow_non_pypi)?;
            let name_from_url = url
                .filename()
                .is_ok_and(|filename| filename == *file.filename);
            let url = Some(url.with_policy(Raw));
            let upload_time = file
                .upload_time_utc_ms
                .map(Timestamp::from_millisecond)
                .transpose()
                .map_err(PylockTomlError::from)?;
            let hashes = Hashes::from(file.hashes.clone());
            if hashes.is_empty() {
                return Err(PublishLockError::MissingHashes);
            }
            match filename {
                DistFilename::SourceDistFilename(_) => {
                    if sdist.is_some() {
                        return Err(PublishLockError::MultipleSourceDistributions(name.clone()));
                    }
                    sdist = Some(PylockTomlSdist {
                        name: (!name_from_url).then(|| file.filename.clone()),
                        url,
                        path: None,
                        upload_time,
                        size: file.size,
                        hashes,
                    });
                }
                DistFilename::WheelFilename(filename) => {
                    if filename.variant().is_some() {
                        return Err(PylockTomlError::from(
                            PylockTomlErrorKind::WheelVariantUnsupported(name.clone()),
                        )
                        .into());
                    }
                    wheels.push(PylockTomlWheel {
                        name: (!name_from_url).then(|| filename.clone()),
                        url,
                        path: None,
                        upload_time,
                        size: file.size,
                        hashes,
                    });
                }
            }
        }

        let package = self
            .packages
            .iter_mut()
            .find(|package| &package.name == name && package.directory.is_some())
            .ok_or_else(|| PublishLockError::MissingWorkspaceDependency(name.clone()))?;
        package.version = Some(version.clone());
        package.index = Some(index.clone().into_url());
        package.directory = None;
        package.sdist = sdist;
        package.wheels = (!wheels.is_empty()).then_some(wheels);
        Ok(())
    }
}

fn is_pypi(url: &SafeUrl) -> bool {
    url.as_str().trim_end_matches('/') == PYPI_URL.as_str()
}

fn check_url(
    package: &PackageName,
    filename: &str,
    url: &SafeUrl,
    allow_non_pypi: bool,
) -> Result<(), PublishLockError> {
    if is_publish_url(url, allow_non_pypi) {
        return Ok(());
    }
    Err(PublishLockError::ArtifactUrl {
        package: package.clone(),
        filename: filename.to_string(),
        url: url.clone(),
    })
}

fn is_publish_url(url: &SafeUrl, allow_non_pypi: bool) -> bool {
    url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && if allow_non_pypi {
            matches!(url.scheme(), "http" | "https")
        } else {
            url.scheme() == "https" && !url.as_str().contains('?')
        }
}

impl Lock {
    /// Compare a workspace member's current dependency declarations with its recorded metadata.
    pub fn matches_publish_requirements(
        &self,
        root: &Path,
        package: &Package,
        requires_dist: &[Requirement],
        provides_extra: &[ExtraName],
    ) -> Result<bool, LockError> {
        let expected: BTreeSet<_> = provides_extra.iter().collect();
        let actual: BTreeSet<_> = package.metadata.provides_extra.iter().collect();
        Ok((!self.supports_provides_extra() || expected == actual)
            && Self::matches_publish_declarations(
                root,
                &self.requires_python,
                requires_dist.iter().cloned(),
                package.metadata.requires_dist.iter().cloned(),
            )?)
    }

    /// Compare dependency declarations after normalizing paths, specifiers, and Python markers.
    pub fn matches_publish_declarations(
        root: &Path,
        requires_python: &RequiresPython,
        left: impl IntoIterator<Item = Requirement>,
        right: impl IntoIterator<Item = Requirement>,
    ) -> Result<bool, LockError> {
        let normalizer = RequirementNormalizer::new(root, requires_python);
        Ok(normalizer.requirements(left)? == normalizer.requirements(right)?)
    }
}

struct PublishTarget<'lock> {
    root: &'lock Path,
    lock: &'lock Lock,
    name: &'lock PackageName,
}

impl<'lock> Installable<'lock> for PublishTarget<'lock> {
    fn install_path(&self) -> &'lock Path {
        self.root
    }

    fn lock(&self) -> &'lock Lock {
        self.lock
    }

    fn roots(&self) -> impl Iterator<Item = &PackageName> {
        std::iter::once(self.name)
    }

    fn project_name(&self) -> Option<&PackageName> {
        Some(self.name)
    }
}
