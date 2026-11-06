use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::path::Path;

use jiff::Timestamp;

use uv_configuration::{
    DependencyGroupsWithDefaults, ExcludeDependency, ExtrasSpecification, InstallOptions, Override,
};
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
    #[error("Cannot export an index URL with a query string")]
    IndexQuery,
    #[error("Cannot export `{0}` from `uv.lock`: workspace dependency `{1}` has no locked version")]
    MissingWorkspaceVersion(PackageName, PackageName),
    #[error("Cannot export `{0}` from `uv.lock`: dependency `{1}` has no registry artifacts")]
    MissingArtifacts(PackageName, PackageName),
    #[error("No published files found for `{0}=={1}`")]
    MissingPublishedFiles(PackageName, Version),
    #[error(
        "Locked workspace publishing requires a package index to look up uploaded files; pass `--check-url` or `--index`"
    )]
    MissingPublishIndex,
    #[error("Cannot export `{0}` from `uv.lock`: the resolution does not satisfy `{1}`")]
    UnsatisfiedRequirement(PackageName, Box<Requirement>),
    #[error(
        "Cannot export `{0}` from `uv.lock`: dependency overrides, exclusions, or metadata overrides can change its published requirements"
    )]
    RuntimeModifiers(PackageName),
    #[error("Cannot export a lock with missing artifact hashes; regenerate `uv.lock`")]
    MissingHashes,
    #[error("Cannot export artifact `{filename}` for `{package}` with an unsupported URL: `{url}`")]
    ArtifactUrl {
        package: PackageName,
        filename: String,
        url: SafeUrl,
    },
    #[error("Cannot export artifact `{filename}` for `{package}` with a query string in its URL")]
    ArtifactQuery {
        package: PackageName,
        filename: String,
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

/// A runtime lock export awaiting registry artifacts for its workspace dependencies.
#[derive(Debug)]
pub struct PublishLock {
    pylock: PylockToml,
    workspace_dependencies: BTreeMap<PackageName, Version>,
    allow_non_pypi: bool,
}

impl PublishLock {
    /// Prepare a runtime-only export and identify its reachable workspace dependencies.
    pub fn from_lock(
        lock: &Lock,
        root: &Path,
        project: &ResolutionMetadata,
        allow_non_pypi: bool,
    ) -> Result<Self, PublishLockError> {
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
                if has_query(index.url()) {
                    return Err(PublishLockError::IndexQuery);
                }
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

        // Registry package metadata is not generally recorded in the lock. Global modifiers can
        // remove all edges to their targets, so an absent target is insufficient to prove safety.
        let reachable = |package: &PackageName| {
            package == name || nodes.iter().any(|node| node.package.name() == package)
        };
        if lock.manifest.overrides.iter().any(|entry| match entry {
            Override::Requirement(_) => true,
            Override::Package(scope) => reachable(scope.package.name()),
        }) || lock.manifest.excludes.iter().any(|entry| match entry {
            ExcludeDependency::Dependency(_) => true,
            ExcludeDependency::Package(scope) => reachable(scope.package()),
        }) || lock
            .manifest
            .dependency_metadata
            .iter()
            .any(|entry| reachable(&entry.name))
        {
            return Err(PublishLockError::RuntimeModifiers(name.clone()));
        }

        let pylock = PylockToml::from_lock(
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
                let filename = sdist
                    .filename(&package.name)
                    .map_err(PylockTomlError::from)?;
                check_url(&package.name, filename, url.as_url(), allow_non_pypi)?;
            }
            for wheel in package.wheels.iter().flatten() {
                if let Some(url) = &wheel.url {
                    let filename = wheel
                        .filename(&package.name)
                        .map_err(PylockTomlError::from)?;
                    check_url(&package.name, filename, url.as_url(), allow_non_pypi)?;
                }
            }
        }
        if pylock.has_missing_hashes() {
            return Err(PublishLockError::MissingHashes);
        }

        Ok(Self {
            pylock,
            workspace_dependencies: workspace,
            allow_non_pypi,
        })
    }

    /// The workspace dependencies and versions that must be available before finalization.
    pub fn workspace_dependencies(&self) -> &BTreeMap<PackageName, Version> {
        &self.workspace_dependencies
    }

    /// Replace every workspace entry with its published registry artifacts before serialization.
    pub fn finalize(
        mut self,
        index: Option<&IndexUrl>,
        published: &BTreeMap<PackageName, Vec<File>>,
    ) -> Result<PylockToml, PublishLockError> {
        if self.workspace_dependencies.is_empty() {
            return Ok(self.pylock);
        }
        let index = index.ok_or(PublishLockError::MissingPublishIndex)?;
        Self::check_publish_index(index, self.allow_non_pypi)?;
        for (name, version) in &self.workspace_dependencies {
            let files = published
                .get(name)
                .filter(|files| !files.is_empty())
                .ok_or_else(|| {
                    PublishLockError::MissingPublishedFiles(name.clone(), version.clone())
                })?;
            Self::replace_dependency(
                &mut self.pylock,
                name,
                version,
                index,
                files,
                self.allow_non_pypi,
            )?;
        }
        Ok(self.pylock)
    }

    /// Validate the destination index before exporting any workspace artifacts.
    pub fn check_publish_index(
        index: &IndexUrl,
        allow_non_pypi: bool,
    ) -> Result<(), PublishLockError> {
        if has_query(index.url()) {
            return Err(PublishLockError::IndexQuery);
        }
        if !allow_non_pypi && !is_pypi(index.url()) {
            return Err(PublishLockError::NonPypiIndex(index.url().clone()));
        }
        if !is_publish_url(index.url(), allow_non_pypi) {
            return Err(PublishLockError::InvalidIndex(index.url().clone()));
        }
        Ok(())
    }

    fn replace_dependency(
        pylock: &mut PylockToml,
        name: &PackageName,
        version: &Version,
        index: &IndexUrl,
        files: &[File],
        allow_non_pypi: bool,
    ) -> Result<(), PublishLockError> {
        let mut sdist = None;
        let mut wheels = Vec::new();
        for file in files {
            let filename = DistFilename::try_from_normalized_filename(&file.filename)
                .filter(|filename| filename.name() == name && filename.version() == version)
                .ok_or_else(|| {
                    PublishLockError::ArtifactIdentity(
                        file.filename.to_string(),
                        name.clone(),
                        version.clone(),
                    )
                })?;
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
                        name: (!name_from_url).then_some(filename),
                        url,
                        path: None,
                        upload_time,
                        size: file.size,
                        hashes,
                    });
                }
            }
        }

        let package = pylock
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
    filename: impl Display,
    url: &SafeUrl,
    allow_non_pypi: bool,
) -> Result<(), PublishLockError> {
    if has_query(url) {
        return Err(PublishLockError::ArtifactQuery {
            package: package.clone(),
            filename: filename.to_string(),
        });
    }
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
            url.scheme() == "https"
        }
}

fn has_query(url: &SafeUrl) -> bool {
    url.as_str()
        .split_once('?')
        .is_some_and(|(prefix, _)| !prefix.contains('#'))
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
