//! Project Python requests and compatibility validation.

use std::path::Path;

use crate::ConfigDiscovery;
use crate::PythonInstallation;
use crate::PythonVersionFile;
use crate::VersionFileDiscoveryOptions;
use itertools::Itertools;
use tracing::debug;
use uv_cache::Cache;
use uv_client::BaseClientBuilder;
use uv_configuration::{DependencyGroupsWithDefaults, NoSources, TargetTriple};
use uv_distribution_types::RequiresPython;
use uv_fs::Simplified;
use uv_pep440::TildeVersionSpecifier;
use uv_pep508::{MarkerEnvironment, MarkerTree};
use uv_python_interpreter::{Interpreter, RequestedInterpreter};
use uv_python_types::{
    EnvironmentPreference, PythonArchitecture, PythonDownloads, PythonPreference, PythonRequest,
};
use uv_settings::PythonInstallMirrors;
use uv_warnings::warn_user_once;
use uv_workspace::{RequiresPythonDeclaration, RequiresPythonSources, Workspace};

use crate::PythonDownloadReporter;
use crate::PythonSelectionError;

/// An interpreter that satisfies the Python requirement used to select it.
///
/// Created by [`ProjectPythonRequest::validate`] after checking the workspace or frozen lockfile
/// requirement, including the selected dependency groups. Warning-only commands and existing
/// environments preserved by `--no-sync` do not use this type.
#[derive(Debug)]
pub struct CompatibleProjectPython(RequestedInterpreter);

impl CompatibleProjectPython {
    /// Consume the compatible interpreter for use by the environment or resolver APIs.
    pub fn into_interpreter(self) -> Interpreter {
        self.0.into_interpreter()
    }

    /// Retain the resolved request for environment creation.
    pub fn into_requested_interpreter(self) -> RequestedInterpreter {
        self.0
    }
}

#[derive(Debug, Clone)]
pub enum PythonRequestSource {
    /// The request was provided by the user.
    UserRequest,
    /// The request was inferred from a `.python-version` or `.python-versions` file.
    DotPythonVersion(PythonVersionFile),
    /// The request was inferred from a `pyproject.toml` file.
    RequiresPython,
}

impl std::fmt::Display for PythonRequestSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UserRequest => write!(f, "explicit request"),
            Self::DotPythonVersion(file) => {
                write!(f, "version file at `{}`", file.path().user_display())
            }
            Self::RequiresPython => write!(f, "`requires-python` metadata"),
        }
    }
}

/// A Python requirement and the source used to derive it.
#[derive(Debug, Clone)]
pub struct ProjectPythonRequirement {
    pub requires_python: RequiresPython,
    /// The complete selected domain, retained separately from its universal Python projection.
    pub environments: MarkerTree,
    pub source: PythonRequirementSource,
}

/// The resolved Python request and requirement for a workspace or frozen lockfile.
#[derive(Debug, Clone)]
pub struct ProjectPythonRequest {
    /// The source of the Python request.
    source: PythonRequestSource,
    /// The resolved Python request, computed by considering (1) any explicit request from the user
    /// via `--python`, (2) any implicit request from the user via `.python-version`, and (3) the
    /// workspace or lockfile's `Requires-Python` specifier.
    pub python_request: Option<PythonRequest>,
    /// The resolved Python requirement for the project and its source.
    requirement: Option<ProjectPythonRequirement>,
}

impl ProjectPythonRequest {
    /// Determine the [`ProjectPythonRequest`] for the current [`Workspace`].
    pub async fn from_request(
        python_request: Option<PythonRequest>,
        workspace: Option<&Workspace>,
        groups: &DependencyGroupsWithDefaults,
        sources: &NoSources,
        project_dir: &Path,
        config_discovery: ConfigDiscovery,
    ) -> Result<Self, PythonSelectionError> {
        let requirement = workspace
            .map(|workspace| find_workspace_python_requirement(workspace, groups, sources))
            .transpose()?
            .flatten();

        Self::from_requirements(
            python_request,
            workspace.map(|workspace| workspace.install_path().as_path()),
            requirement,
            project_dir,
            config_discovery,
        )
        .await
    }

    /// Select a Python request using a project's root and Python requirement.
    pub async fn from_requirements(
        python_request: Option<PythonRequest>,
        workspace_root: Option<&Path>,
        requirement: Option<ProjectPythonRequirement>,
        project_dir: &Path,
        config_discovery: ConfigDiscovery,
    ) -> Result<Self, PythonSelectionError> {
        let (source, python_request) = if let Some(request) = python_request {
            // (1) Explicit request from user
            let source = PythonRequestSource::UserRequest;
            let request = Some(request);
            (source, request)
        } else if let Some(file) = PythonVersionFile::discover(
            project_dir,
            &VersionFileDiscoveryOptions::default()
                .with_stop_discovery_at(workspace_root)
                .with_config_discovery(config_discovery),
        )
        .await?
        .filter(|file| {
            // Ignore global version files that are incompatible with requires-python
            if !file.is_global() {
                return true;
            }
            match (file.version(), requirement.as_ref()) {
                (Some(request), Some(requirement)) => request
                    .as_pep440_version()
                    .is_none_or(|version| requirement.requires_python.contains(&version)),
                _ => true,
            }
        }) {
            // (2) Request from `.python-version`
            let source = PythonRequestSource::DotPythonVersion(file.clone());
            let request = file.version().cloned();
            (source, request)
        } else {
            // (3) `requires-python` in `pyproject.toml`
            let request = requirement.as_ref().and_then(|requirement| {
                PythonRequest::from_specifiers(requirement.requires_python.specifiers())
            });
            let source = PythonRequestSource::RequiresPython;
            (source, request)
        };

        if let Some(python_request) = python_request.as_ref() {
            debug!(
                "Using Python request `{}` from {source}",
                python_request.to_canonical_string()
            );
        }

        Ok(Self {
            source,
            python_request,
            requirement,
        })
    }

    pub fn requires_python(&self) -> Option<&RequiresPython> {
        self.requirement
            .as_ref()
            .map(|requirement| &requirement.requires_python)
    }

    /// Whether the selected domain retains conditions beyond its Python projection.
    pub fn has_environment_constraints(&self) -> bool {
        self.requirement.as_ref().is_some_and(|requirement| {
            let selected = requirement
                .requires_python
                .to_exact_marker_tree()
                .and(requirement.environments);
            let python = RequiresPython::from_marker_tree_parts(selected)
                .iter()
                .fold(MarkerTree::FALSE, |marker, requirement| {
                    marker.or(requirement.to_exact_marker_tree())
                });
            // Pure Python environment filters are validated by the lock. Discovery refines the
            // platform conditions that the universal Python projection cannot express.
            selected != python
        })
    }

    /// Defer an unqualified numeric global default while probing its platform compatibility.
    #[must_use]
    pub fn environment_probe(&self) -> Self {
        let mut probe = self.clone();
        if matches!(&self.source, PythonRequestSource::DotPythonVersion(file)
            if file.is_global())
            && let Some(PythonRequest::Version(_)) = self.python_request.as_ref()
            && self
                .python_request
                .as_ref()
                .and_then(PythonRequest::as_pep440_version)
                .is_some()
        {
            probe.source = PythonRequestSource::RequiresPython;
            probe.python_request = self.requirement.as_ref().and_then(|requirement| {
                PythonRequest::from_specifiers(requirement.requires_python.specifiers())
            });
        }
        probe
    }

    /// Keep only the universal Python projection for a command that skips synchronization.
    #[must_use]
    pub fn without_environment_constraints(mut self) -> Self {
        if let Some(requirement) = self.requirement.as_mut() {
            requirement.environments = MarkerTree::TRUE;
        }
        self
    }

    /// Derive exact Python interval choices for a concrete platform without fixing its version.
    pub fn for_environment(
        self,
        environment: &MarkerEnvironment,
    ) -> Result<Vec<Self>, PythonSelectionError> {
        let Some(requirement) = self.requirement.as_ref() else {
            return Ok(vec![self]);
        };
        let selected = requirement.requires_python.to_exact_marker_tree().and(
            requirement
                .environments
                .evaluate_environment_strings(environment),
        );
        let mut requirements = match RequiresPython::from_marker_tree(selected) {
            Some(requirement) => vec![requirement],
            None => RequiresPython::from_marker_tree_parts(selected),
        };
        if requirements.is_empty() {
            return Err(PythonSelectionError::UnsupportedEnvironment(
                requirement
                    .environments
                    .contents()
                    .expect("an unsupported environment is not universally true"),
            ));
        }
        let version = &environment.python_full_version().version;
        // Retain a compatible probe before considering another interval. Otherwise try the newest
        // interval first; callers check all installed alternatives before attempting a download.
        requirements.reverse();
        requirements.sort_by_key(|requirement| !requirement.contains(version));

        let global_version = match (&self.source, self.python_request.as_ref()) {
            (
                PythonRequestSource::DotPythonVersion(file),
                Some(PythonRequest::Version(version)),
            ) if file.is_global()
                && self
                    .python_request
                    .as_ref()
                    .and_then(PythonRequest::as_pep440_version)
                    .is_some() =>
            {
                Some(version)
            }
            _ => None,
        };
        let implicit = if global_version.is_some() {
            let compatible = requirements
                .iter()
                .filter(|requirement| {
                    self.python_request.as_ref().is_some_and(|request| {
                        request.intersects_specifiers(requirement.specifiers())
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            if compatible.is_empty() {
                true
            } else {
                requirements = compatible;
                false
            }
        } else {
            match &self.source {
                PythonRequestSource::RequiresPython => true,
                PythonRequestSource::DotPythonVersion(_) | PythonRequestSource::UserRequest => {
                    false
                }
            }
        };
        if !implicit && global_version.is_none() && requirements.len() > 1 {
            let Some(index) = requirements
                .iter()
                .position(|requirement| requirement.contains(version))
            else {
                return Err(PythonSelectionError::UnsupportedEnvironment(
                    requirement
                        .environments
                        .contents()
                        .expect("a disjoint environment is not universally true"),
                ));
            };
            let requirement = requirements.swap_remove(index);
            requirements = vec![requirement];
        }
        Ok(requirements
            .into_iter()
            .map(|requires_python| {
                let mut selected = self.clone();
                selected
                    .requirement
                    .as_mut()
                    .expect("a selected requirement exists")
                    .requires_python = requires_python;
                if !implicit && let Some(version) = global_version {
                    selected.python_request = Some(PythonRequest::Version(
                        version.intersect_specifiers(
                            selected
                                .requirement
                                .as_ref()
                                .expect("a selected requirement exists")
                                .requires_python
                                .specifiers(),
                        ),
                    ));
                } else if implicit {
                    selected.source = PythonRequestSource::RequiresPython;
                    selected.python_request = PythonRequest::from_specifiers(
                        selected
                            .requirement
                            .as_ref()
                            .expect("a selected requirement exists")
                            .requires_python
                            .specifiers(),
                    );
                }
                selected
            })
            .collect())
    }

    /// Prefer a compatible existing interpreter across all interval alternatives before downloading.
    pub fn prefer_existing(
        requests: &mut [Self],
        probe: &Interpreter,
        environments: EnvironmentPreference,
        preference: PythonPreference,
        arch: Option<PythonArchitecture>,
        cache: &Cache,
    ) -> Result<(), PythonSelectionError> {
        if let Some(index) = requests.iter().position(|request| {
            probe.matches_request(
                request
                    .python_request
                    .as_ref()
                    .unwrap_or(&PythonRequest::Default),
                cache,
            ) && request.check(probe).is_ok()
        }) {
            requests.swap(0, index);
            return Ok(());
        }
        for (index, request) in requests.iter().enumerate() {
            match PythonInstallation::find_existing(
                request
                    .python_request
                    .as_ref()
                    .unwrap_or(&PythonRequest::Default),
                environments,
                preference,
                arch,
                cache,
            ) {
                Ok(installation) if request.check(installation.interpreter()).is_ok() => {
                    requests.swap(0, index);
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) if error.can_try_another_request() => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Check the interpreter against the stored project and selected group requirements.
    ///
    /// Unlike [`Self::validate`], this borrows the interpreter so warning-only commands can
    /// continue using it after an incompatibility.
    pub fn check(&self, interpreter: &Interpreter) -> Result<(), PythonSelectionError> {
        let Some(requirement) = &self.requirement else {
            return Ok(());
        };
        validate_python_requirement(
            interpreter,
            &requirement.requires_python,
            &self.source,
            &requirement.source,
        )
    }

    /// Validate a discovered interpreter before accepting it for a new project environment.
    ///
    /// Discovery is responsible for matching the Python request; this checks the project and
    /// selected group requirements.
    pub fn validate(
        &self,
        interpreter: Interpreter,
    ) -> Result<CompatibleProjectPython, PythonSelectionError> {
        self.check(&interpreter)?;
        Ok(CompatibleProjectPython(RequestedInterpreter::new(
            interpreter,
            self.python_request.clone().unwrap_or_default(),
        )))
    }

    /// Find or download an interpreter for a concrete project environment.
    ///
    /// Platform-dependent bounds can refine an implicit request before final selection. Explicit
    /// requests and local version files remain constraints on that selection.
    pub async fn find_or_download(
        &self,
        environment_preference: EnvironmentPreference,
        python_preference: PythonPreference,
        python_arch: Option<PythonArchitecture>,
        python_platform: Option<&TargetTriple>,
        python_downloads: PythonDownloads,
        client_builder: &BaseClientBuilder<'_>,
        cache: &Cache,
        reporter: &PythonDownloadReporter,
        install_mirrors: &PythonInstallMirrors,
    ) -> Result<CompatibleProjectPython, PythonSelectionError> {
        let (selected, installation) = self
            .find_or_download_for_environment(
                environment_preference,
                python_preference,
                python_arch,
                python_platform,
                python_downloads,
                client_builder,
                cache,
                reporter,
                install_mirrors,
                Err,
            )
            .await?;
        selected.validate(installation.into_interpreter())
    }

    /// Search for a platform-compatible installation and retain the request that selected it.
    ///
    /// The handler determines whether an unsupported environment is fatal or the original request
    /// can still be used. Strict callers stop before retrying the original request. Callers
    /// validate or warn about the returned installation's Python requirement separately.
    pub async fn find_or_download_for_environment(
        &self,
        environment_preference: EnvironmentPreference,
        python_preference: PythonPreference,
        python_arch: Option<PythonArchitecture>,
        python_platform: Option<&TargetTriple>,
        python_downloads: PythonDownloads,
        client_builder: &BaseClientBuilder<'_>,
        cache: &Cache,
        reporter: &PythonDownloadReporter,
        install_mirrors: &PythonInstallMirrors,
        on_unsupported_environment: impl FnOnce(
            PythonSelectionError,
        ) -> Result<(), PythonSelectionError>,
    ) -> Result<(Self, PythonInstallation), PythonSelectionError> {
        let probe_request = if self.has_environment_constraints() {
            self.environment_probe()
        } else {
            self.clone()
        };
        let probe = PythonInstallation::find_or_download(
            probe_request.python_request.as_ref(),
            environment_preference,
            python_preference,
            python_arch,
            python_downloads,
            client_builder,
            cache,
            Some(reporter),
            install_mirrors.mirrors(),
            install_mirrors.python_downloads_json_url.as_deref(),
        )
        .await?;
        if !self.has_environment_constraints() {
            return Ok((self.clone(), probe));
        }
        let markers = python_platform.map_or_else(
            || probe.interpreter().markers().clone(),
            |platform| platform.markers(probe.interpreter().markers().clone()),
        );
        let requests = match self.clone().for_environment(&markers) {
            Ok(mut requests) => {
                Self::prefer_existing(
                    &mut requests,
                    probe.interpreter(),
                    environment_preference,
                    python_preference,
                    python_arch,
                    cache,
                )?;
                requests
            }
            Err(error) => {
                on_unsupported_environment(error)?;
                vec![self.clone()]
            }
        };
        let mut missing = None;
        for selected in requests {
            match PythonInstallation::find_or_download(
                selected.python_request.as_ref(),
                environment_preference,
                python_preference,
                python_arch,
                python_downloads,
                client_builder,
                cache,
                Some(reporter),
                install_mirrors.mirrors(),
                install_mirrors.python_downloads_json_url.as_deref(),
            )
            .await
            {
                Ok(installation) => return Ok((selected, installation)),
                Err(error) if error.can_try_another_request() => missing = Some(error),
                Err(error) => return Err(error.into()),
            }
        }
        Err(missing
            .expect("at least one environment request was attempted")
            .into())
    }
}

/// Compute the `Requires-Python` bound for the [`Workspace`].
///
/// For a [`Workspace`] with multiple packages, the `Requires-Python` bound is the union of the
/// `Requires-Python` bounds of all the packages.
pub fn find_requires_python(
    workspace: &Workspace,
    groups: &DependencyGroupsWithDefaults,
    sources: &NoSources,
) -> Result<Option<RequiresPython>, PythonSelectionError> {
    Ok(
        find_workspace_python_requirement(workspace, groups, sources)?
            .map(|requirement| requirement.requires_python),
    )
}

/// Compute the workspace's Python requirement together with its contributing declarations.
///
/// Retain the declarations so incompatibility diagnostics use the same inputs as the requirement.
fn find_workspace_python_requirement(
    workspace: &Workspace,
    groups: &DependencyGroupsWithDefaults,
    sources: &NoSources,
) -> Result<Option<ProjectPythonRequirement>, PythonSelectionError> {
    let requires_python = workspace.requires_python(groups)?;
    if let Some(workspace_group_requires_python) =
        workspace.workspace_group_requires_python(sources)?
    {
        let Some(requires_python_intersection) = RequiresPython::intersection(
            std::iter::once(workspace_group_requires_python.specifiers()).chain(
                requires_python
                    .iter()
                    .filter_map(|(declaration, specifiers)| match declaration {
                        RequiresPythonDeclaration::Member(_, Some(_))
                        | RequiresPythonDeclaration::Workspace(_) => Some(specifiers),
                        RequiresPythonDeclaration::Member(_, None) => None,
                    }),
            ),
        ) else {
            return Err(PythonSelectionError::DisjointRequiresPython(
                requires_python,
            ));
        };
        return Ok(Some(ProjectPythonRequirement {
            requires_python: requires_python_intersection,
            environments: workspace
                .environments()
                .filter(|environments| !environments.is_empty())
                .map_or(MarkerTree::TRUE, |environments| {
                    environments
                        .iter()
                        .copied()
                        .fold(MarkerTree::FALSE, MarkerTree::or)
                }),
            source: PythonRequirementSource::Workspace {
                sources: requires_python,
                multiple_members: workspace.packages().len() > 1,
            },
        }));
    }
    // If there are no `Requires-Python` specifiers in the workspace, return `None`.
    if requires_python.is_empty() {
        return Ok(None);
    }
    for (source, specifiers) in &requires_python {
        if let [spec] = &specifiers[..] {
            if let Some(spec) = TildeVersionSpecifier::from_specifier_ref(spec) {
                if spec.has_patch() {
                    continue;
                }
                let (lower, upper) = spec.bounding_specifiers();
                let spec_0 = spec.with_patch_version(0);
                let (lower_0, upper_0) = spec_0.bounding_specifiers();
                warn_user_once!(
                    "The `requires-python` specifier (`{spec}`) in `{source}` \
                    uses the tilde specifier (`~=`) without a patch version. This will be \
                    interpreted as `{lower}, {upper}`. Did you mean `{spec_0}` to constrain the \
                    version as `{lower_0}, {upper_0}`? We recommend only using \
                    the tilde specifier with a patch version to avoid ambiguity.",
                );
            }
        }
    }
    match RequiresPython::intersection(requires_python.iter().map(|(.., specifiers)| specifiers)) {
        Some(intersection) => Ok(Some(ProjectPythonRequirement {
            requires_python: intersection,
            environments: workspace
                .environments()
                .filter(|environments| !environments.is_empty())
                .map_or(MarkerTree::TRUE, |environments| {
                    environments
                        .iter()
                        .copied()
                        .fold(MarkerTree::FALSE, MarkerTree::or)
                }),
            source: PythonRequirementSource::Workspace {
                sources: requires_python,
                multiple_members: workspace.packages().len() > 1,
            },
        })),
        None => Err(PythonSelectionError::DisjointRequiresPython(
            requires_python,
        )),
    }
}

/// The requirements that exclude a Python version, and where they were read.
///
/// Formats as an optional suffix to a Python incompatibility diagnostic.
#[derive(Debug)]
pub enum PythonRequirementConflicts {
    Workspace {
        sources: RequiresPythonSources,
        /// Whether the workspace has multiple members, so a single-conflict diagnostic names the member.
        multiple_members: bool,
    },
    Lockfile {
        locked: Option<RequiresPython>,
        groups: RequiresPythonSources,
    },
}

impl std::fmt::Display for PythonRequirementConflicts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workspace {
                sources,
                multiple_members,
            } => {
                if sources.len() > 1 {
                    return write!(
                        f,
                        ".\nThe following `requires-python` declarations do not permit this version:\n{}",
                        format_requires_python_sources(sources)
                    );
                }
                if let Some((RequiresPythonDeclaration::Workspace(group), _)) =
                    sources.iter().next()
                {
                    return write!(
                        f,
                        " (from the workspace root's `tool.uv.dependency-groups.{group}.requires-python`)."
                    );
                }
                if let Some((RequiresPythonDeclaration::Member(package, group), _)) =
                    sources.iter().next()
                {
                    if let Some(group) = group {
                        if *multiple_members {
                            return write!(
                                f,
                                " (from workspace member `{package}`'s `tool.uv.dependency-groups.{group}.requires-python`)."
                            );
                        }
                        return write!(
                            f,
                            " (from `tool.uv.dependency-groups.{group}.requires-python`)."
                        );
                    }
                    if *multiple_members {
                        return write!(
                            f,
                            " (from workspace member `{package}`'s `project.requires-python`)."
                        );
                    }
                    return f.write_str(" (from `project.requires-python`)");
                }
                Ok(())
            }
            Self::Lockfile { locked, groups } => {
                let count = usize::from(locked.is_some()) + groups.len();
                if count > 1 {
                    write!(
                        f,
                        ".\nThe following requirements in `uv.lock` do not permit this version:\n"
                    )?;
                    if let Some(locked) = locked {
                        writeln!(f, "- lockfile: {locked}")?;
                    }
                    return f.write_str(&format_requires_python_sources(groups));
                }
                if locked.is_some() {
                    return f.write_str(" (from `requires-python` in `uv.lock`).");
                }
                if let Some((source, _)) = groups.iter().next() {
                    return write!(f, " (from `{source}` in `uv.lock`).");
                }
                f.write_str(" (from `uv.lock`).")
            }
        }
    }
}

/// The declarations used to compute a project's Python requirement.
#[derive(Debug, Clone)]
pub enum PythonRequirementSource {
    /// The requirement was derived from the workspace manifests.
    Workspace {
        sources: RequiresPythonSources,
        multiple_members: bool,
    },
    /// The lockfile's overall requirement and the selected groups' requirements.
    Lockfile {
        locked: RequiresPython,
        groups: RequiresPythonSources,
    },
}

/// Returns an error if the [`Interpreter`] does not satisfy `requires_python`.
///
/// The requirement source determines which conflicting declarations are included in the diagnostic.
fn validate_python_requirement(
    interpreter: &Interpreter,
    requires_python: &RequiresPython,
    source: &PythonRequestSource,
    requirement_source: &PythonRequirementSource,
) -> Result<(), PythonSelectionError> {
    if requires_python.contains(interpreter.python_version()) {
        return Ok(());
    }

    let conflicting_requires = match requirement_source {
        PythonRequirementSource::Workspace {
            sources,
            multiple_members,
        } => {
            let sources = sources
                .iter()
                .filter(|(.., requires)| !requires.contains(interpreter.python_version()))
                .map(|(key, requires)| (key.clone(), requires.clone()))
                .collect();
            PythonRequirementConflicts::Workspace {
                sources,
                multiple_members: *multiple_members,
            }
        }
        PythonRequirementSource::Lockfile { locked, groups } => {
            let version = interpreter.python_version().only_release();
            let groups = groups
                .iter()
                .filter(|(_, requires)| !requires.contains(&version))
                .map(|(key, requires)| (key.clone(), requires.clone()))
                .collect();
            PythonRequirementConflicts::Lockfile {
                locked: (!locked.contains(interpreter.python_version())).then(|| locked.clone()),
                groups,
            }
        }
    };

    match source {
        PythonRequestSource::UserRequest => {
            Err(PythonSelectionError::RequestedPythonProjectIncompatibility(
                interpreter.python_version().clone(),
                requires_python.clone(),
                Box::new(conflicting_requires),
            ))
        }
        PythonRequestSource::DotPythonVersion(file) => Err(
            PythonSelectionError::DotPythonVersionProjectIncompatibility {
                python_request: file.path().user_display().to_string(),
                version: interpreter.python_version().clone(),
                requires_python: requires_python.clone(),
                requires_python_sources: Box::new(conflicting_requires),
            },
        ),
        PythonRequestSource::RequiresPython => {
            Err(PythonSelectionError::RequiresPythonProjectIncompatibility(
                interpreter.python_version().clone(),
                requires_python.clone(),
                Box::new(conflicting_requires),
            ))
        }
    }
}

pub fn format_requires_python_sources(conflicts: &RequiresPythonSources) -> String {
    conflicts
        .iter()
        .map(|(source, specifiers)| format!("- {source}: {specifiers}"))
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uv_pep508::MarkerEnvironmentBuilder;

    fn linux_environment() -> MarkerEnvironment {
        MarkerEnvironmentBuilder {
            implementation_name: "cpython",
            implementation_version: "3.12.9",
            os_name: "posix",
            platform_machine: "x86_64",
            platform_python_implementation: "CPython",
            platform_release: "6.12.0",
            platform_system: "Linux",
            platform_version: "1",
            python_full_version: "3.12.9",
            python_version: "3.12",
            sys_platform: "linux",
        }
        .try_into()
        .unwrap()
    }

    fn request(environments: &str) -> ProjectPythonRequest {
        let requires_python = RequiresPython::from_specifiers(">=3.12,<3.14".parse().unwrap());
        ProjectPythonRequest {
            source: PythonRequestSource::RequiresPython,
            python_request: PythonRequest::from_specifiers(requires_python.specifiers()),
            requirement: Some(ProjectPythonRequirement {
                requires_python,
                environments: environments.parse().unwrap(),
                source: PythonRequirementSource::Workspace {
                    sources: RequiresPythonSources::new(),
                    multiple_members: true,
                },
            }),
        }
    }

    #[test]
    fn platform_python_requirement_retains_active_branch() {
        let request = request("sys_platform != 'linux' or python_full_version >= '3.13'");
        let linux = request
            .clone()
            .for_environment(&linux_environment())
            .unwrap()
            .remove(0);
        assert!(
            !linux
                .requires_python()
                .unwrap()
                .contains(&"3.12.9".parse().unwrap())
        );
        assert!(
            linux
                .requires_python()
                .unwrap()
                .contains(&"3.13.1".parse().unwrap())
        );
        let windows = linux_environment()
            .with_os_name("nt")
            .with_platform_system("Windows")
            .with_sys_platform("win32");
        let windows = request.for_environment(&windows).unwrap().remove(0);
        assert!(
            windows
                .requires_python()
                .unwrap()
                .contains(&"3.12.9".parse().unwrap())
        );
    }

    #[test]
    fn platform_python_requirement_evaluates_substring_markers() {
        for marker in [
            "sys_platform not in 'linux darwin' or python_full_version >= '3.13'",
            "sys_platform in 'win32 cygwin' or python_full_version >= '3.13'",
            "'nux' not in sys_platform or python_full_version >= '3.13'",
            "'win' in sys_platform or python_full_version >= '3.13'",
        ] {
            let selected = request(marker)
                .for_environment(&linux_environment())
                .unwrap();
            assert_eq!(selected.len(), 1, "{marker}");
            assert_eq!(
                selected[0]
                    .requires_python()
                    .map(RequiresPython::to_exact_marker_tree),
                Some(
                    RequiresPython::from_specifiers(">=3.13,<3.14".parse().unwrap())
                        .to_exact_marker_tree()
                ),
                "{marker}"
            );
        }
    }

    #[test]
    fn platform_python_requirement_does_not_pin_probe_version() {
        let selected =
            request("python_full_version >= '3.13' and implementation_version >= '3.13'")
                .for_environment(&linux_environment())
                .unwrap()
                .remove(0);
        assert!(
            selected
                .requires_python()
                .unwrap()
                .contains(&"3.13.1".parse().unwrap())
        );
        assert!(
            !selected
                .requires_python()
                .unwrap()
                .contains(&"3.12.9".parse().unwrap())
        );
    }

    #[test]
    fn platform_python_requirement_keeps_explicit_request() {
        let mut request = request("sys_platform != 'linux' or python_full_version >= '3.13'");
        request.source = PythonRequestSource::UserRequest;
        request.python_request = Some(PythonRequest::parse("3.12"));
        let selected = request
            .for_environment(&linux_environment())
            .unwrap()
            .remove(0);
        assert_eq!(
            selected
                .python_request
                .as_ref()
                .unwrap()
                .to_canonical_string(),
            "3.12"
        );
        assert!(matches!(selected.source, PythonRequestSource::UserRequest));
        assert!(
            !selected
                .requires_python()
                .unwrap()
                .contains(&"3.12.9".parse().unwrap())
        );
    }

    #[test]
    fn platform_python_requirement_rejects_unsupported_platform() {
        assert!(matches!(
            request("sys_platform == 'win32'").for_environment(&linux_environment()),
            Err(PythonSelectionError::UnsupportedEnvironment(_))
        ));
    }

    #[test]
    fn platform_python_requirement_retains_disjoint_intervals() {
        let request = request(
            "sys_platform != 'linux' or python_full_version < '3.12.3' or python_full_version >= '3.13'",
        );
        let selected = request.for_environment(&linux_environment()).unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(
            selected[0].requires_python().unwrap().to_string(),
            "==3.13.*"
        );
        assert_eq!(
            selected[1].requires_python().unwrap().to_string(),
            ">=3.12, <3.12.3"
        );
    }

    #[test]
    fn platform_python_requirement_retains_explicit_disjoint_candidate() {
        let mut request = request(
            "sys_platform != 'linux' or python_full_version < '3.12.3' or python_full_version >= '3.13'",
        );
        request.source = PythonRequestSource::UserRequest;
        request.python_request = Some(PythonRequest::parse("3.13"));
        let environment = linux_environment()
            .with_python_full_version("3.13.1".parse::<uv_pep440::Version>().unwrap());
        let selected = request.for_environment(&environment).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].python_request,
            Some(PythonRequest::parse("3.13"))
        );
        assert!(
            selected[0]
                .requires_python()
                .unwrap()
                .contains(&"3.13.1".parse().unwrap())
        );
    }
}
