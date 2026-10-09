use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::str::FromStr;

use itertools::Either;
use rustc_hash::FxHashSet;

use uv_configuration::{
    BuildOptions, Constraints, DependencyGroupsWithDefaults, ExtrasSpecification,
    ExtrasSpecificationWithDefaults, InstallOptions, InstallTarget as InstallOptionTarget,
};
use uv_distribution_types::{Index, RequiresPython, Resolution};
use uv_lock::{
    Installable, InstallableRootKind, Lock, LockError, Package, implicit_constraints_marker,
};
use uv_normalize::{DEV_DEPENDENCIES, ExtraName, GroupName, PackageName};
use uv_pep440::{Version, VersionSpecifier};
use uv_pep508::{MarkerExpression, MarkerTree, MarkerValueVersion};
use uv_platform_tags::Tags;
use uv_pypi_types::{
    ConflictItem, DependencyGroupSpecifier, DependencyGroups, LenientRequirement,
    ResolverMarkerEnvironment, VerbatimParsedUrl,
};
use uv_python_discovery::ProjectPythonRequirement;
use uv_python_discovery::PythonRequirementSource;
use uv_scripts::Pep723Script;
use uv_workspace::pyproject::{Source, Sources, ToolUvSources};
use uv_workspace::{RequiresPythonDeclaration, RequiresPythonSources, VirtualProject, Workspace};

use crate::EnvironmentError;

/// A target that can be installed from a lockfile.
#[derive(Debug, Copy, Clone)]
pub enum InstallTarget<'lock> {
    /// A project (which could be a workspace root or member).
    Project {
        workspace: &'lock Workspace,
        name: &'lock PackageName,
        lock: &'lock Lock,
    },
    /// Multiple specific projects in a workspace.
    Projects {
        workspace: &'lock Workspace,
        names: &'lock [PackageName],
        lock: &'lock Lock,
    },
    /// An entire workspace.
    Workspace {
        workspace: &'lock Workspace,
        project_name: Option<&'lock PackageName>,
        lock: &'lock Lock,
    },
    /// An entire workspace with a non-project root.
    NonProjectWorkspace {
        workspace: &'lock Workspace,
        lock: &'lock Lock,
    },
    /// A frozen lockfile without a workspace manifest.
    Lockfile {
        root: &'lock Path,
        project_name: Option<&'lock PackageName>,
        selection: PackageSelection<'lock>,
        lock: &'lock Lock,
    },
    /// A PEP 723 script.
    Script {
        script: &'lock Pep723Script,
        lock: &'lock Lock,
    },
}

/// The workspace packages selected by an installation target.
#[derive(Debug, Copy, Clone)]
pub enum PackageSelection<'lock> {
    Projects(&'lock [PackageName]),
    Workspace,
    NonProjectWorkspace,
}

impl<'lock> PackageSelection<'lock> {
    /// Resolve package flags, defaulting to the current project or non-project workspace.
    pub fn from_args(
        all_packages: bool,
        names: &'lock [PackageName],
        project_name: Option<&'lock PackageName>,
    ) -> Self {
        if all_packages {
            Self::Workspace
        } else if !names.is_empty() {
            Self::Projects(names)
        } else if let Some(name) = project_name {
            Self::Projects(std::slice::from_ref(name))
        } else {
            Self::NonProjectWorkspace
        }
    }

    /// Identify workspace members excluded from installation before a lockfile is available.
    pub fn first_party_exclusions(
        self,
        workspace: &Workspace,
        project_name: Option<&PackageName>,
        install_options: &InstallOptions,
    ) -> BTreeSet<PackageName> {
        let members = workspace
            .packages()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let project_name = match self {
            Self::Projects([name]) => Some(name),
            Self::Projects(_) => None,
            Self::Workspace | Self::NonProjectWorkspace => project_name,
        };
        members
            .iter()
            .filter(|name| {
                !install_options.include_package(
                    InstallOptionTarget {
                        name,
                        is_local: true,
                    },
                    project_name,
                    &members,
                )
            })
            .cloned()
            .collect()
    }
}

impl<'lock> Installable<'lock> for InstallTarget<'lock> {
    fn install_path(&self) -> &'lock Path {
        match self {
            Self::Project { workspace, .. } => workspace.install_path(),
            Self::Projects { workspace, .. } => workspace.install_path(),
            Self::Workspace { workspace, .. } => workspace.install_path(),
            Self::NonProjectWorkspace { workspace, .. } => workspace.install_path(),
            Self::Lockfile { root, .. } => root,
            Self::Script { script, .. } => script.path.parent().unwrap(),
        }
    }

    fn lock(&self) -> &'lock Lock {
        match self {
            Self::Project { lock, .. } => lock,
            Self::Projects { lock, .. } => lock,
            Self::Workspace { lock, .. } => lock,
            Self::NonProjectWorkspace { lock, .. } => lock,
            Self::Lockfile { lock, .. } => lock,
            Self::Script { lock, .. } => lock,
        }
    }

    #[allow(refining_impl_trait)]
    fn roots(&self) -> Box<dyn Iterator<Item = &PackageName> + '_> {
        let lock = self.lock();
        match self.package_selection() {
            Some(PackageSelection::Projects(names)) => Box::new(names.iter()),
            Some(PackageSelection::NonProjectWorkspace) => Box::new(lock.members().iter()),
            Some(PackageSelection::Workspace) => {
                // Identify the workspace members.
                //
                // The members are encoded directly in the lockfile, unless the workspace contains a
                // single member at the root, in which case, we identify it by its source.
                if lock.members().is_empty() {
                    Box::new(lock.root().into_iter().map(Package::name))
                } else {
                    Box::new(lock.members().iter())
                }
            }
            None => Box::new(std::iter::empty()),
        }
    }

    fn group_root(&self, groups: &DependencyGroupsWithDefaults) -> Option<&PackageName> {
        let (name, workspace) = self.selected_project()?;
        let root = self.lock().root().filter(|root| root.name() != name)?;
        let includes_root_group = if let Some(workspace) = workspace {
            let root_member = workspace.packages().get(root.name())?;
            let pyproject = root_member.pyproject_toml();
            let declared_groups = pyproject
                .dependency_groups
                .as_ref()
                .into_iter()
                .flat_map(DependencyGroups::keys);
            let legacy_dev = pyproject
                .tool
                .as_ref()
                .and_then(|tool| tool.uv.as_ref())
                .and_then(|uv| uv.dev_dependencies.as_ref())
                .is_some()
                .then_some(&*DEV_DEPENDENCIES);

            declared_groups
                .chain(legacy_dev)
                .any(|group| self.includes_group(Some(root.name()), group, groups))
        } else {
            root.dependency_groups()
                .keys()
                .chain(root.resolved_dependency_groups().keys())
                .any(|group| self.includes_group(Some(root.name()), group, groups))
        };
        includes_root_group.then_some(root.name())
    }

    fn includes_group(
        &self,
        package: Option<&PackageName>,
        group: &GroupName,
        groups: &DependencyGroupsWithDefaults,
    ) -> bool {
        if !groups.contains(group) {
            return false;
        }

        let Some((name, workspace)) = self.selected_project() else {
            return true;
        };

        if package == Some(name) {
            return true;
        }

        // Workspace-root groups must be requested explicitly when a member is selected.
        // Defaults belong to the selected member, not to an inherited workspace root.
        if groups.contains_because_default(group) {
            return false;
        }

        let Some(workspace) = workspace else {
            return !self
                .lock()
                .find_by_name(name)
                .ok()
                .flatten()
                .is_some_and(|member| {
                    member.dependency_groups().contains_key(group)
                        || member.resolved_dependency_groups().contains_key(group)
                });
        };
        !workspace.packages().get(name).is_some_and(|member| {
            let pyproject = member.pyproject_toml();
            pyproject
                .dependency_groups
                .as_ref()
                .is_some_and(|member_groups| member_groups.contains_key(group))
                // Legacy development dependencies also define the member's `dev` group, so
                // they take precedence over an inherited `dev` group from the workspace root.
                || group == &*DEV_DEPENDENCIES
                    && pyproject
                        .tool
                        .as_ref()
                        .and_then(|tool| tool.uv.as_ref())
                        .and_then(|uv| uv.dev_dependencies.as_ref())
                        .is_some()
        })
    }

    fn project_name(&self) -> Option<&PackageName> {
        match self {
            Self::Project { name, .. } => Some(name),
            Self::Projects { .. } => None,
            Self::Workspace {
                project_name, lock, ..
            } => project_name.or_else(|| {
                // A single-member workspace omits the member list from the lockfile.
                if lock.members().is_empty() {
                    lock.root().map(Package::name)
                } else {
                    None
                }
            }),
            Self::NonProjectWorkspace { .. } => None,
            Self::Lockfile {
                project_name,
                selection,
                ..
            } => match selection {
                PackageSelection::Projects([name]) => Some(name),
                PackageSelection::Projects(_) => None,
                PackageSelection::Workspace | PackageSelection::NonProjectWorkspace => {
                    *project_name
                }
            },
            Self::Script { .. } => None,
        }
    }
}

impl<'lock> InstallTarget<'lock> {
    /// Intersect the lockfile's Python requirement with selected members, roots, and groups.
    pub fn python_requirement(
        &self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<ProjectPythonRequirement, EnvironmentError> {
        let lock = self.lock();
        let mut group_requirements = RequiresPythonSources::new();
        for package in lock
            .non_root_workspace_packages()
            .filter(|package| groups.prod() && self.roots().any(|root| root == package.name()))
        {
            let requires_python = package.workspace_requires_python().ok_or_else(|| {
                EnvironmentError::MissingWorkspaceMemberPython(package.name().clone())
            })?;
            group_requirements.insert(
                RequiresPythonDeclaration::Member(package.name().clone(), None),
                requires_python.clone(),
            );
        }

        for name in self.roots() {
            let Ok(Some(package)) = lock.find_by_name(name) else {
                // Lockfile selection reports missing or ambiguous roots.
                continue;
            };
            if package.fork_markers().is_empty() {
                continue;
            }
            let marker = package
                .fork_markers()
                .iter()
                .fold(MarkerTree::FALSE, |marker, fork| marker.or(fork.pep508()));
            let Some(requirement) = RequiresPython::from_marker_tree(marker) else {
                return Err(EnvironmentError::DisjointLockedRequiresPython {
                    locked: lock.requires_python().clone(),
                    groups: group_requirements,
                });
            };
            group_requirements
                .entry(RequiresPythonDeclaration::Member(name.clone(), None))
                .and_modify(|specifiers| {
                    *specifiers = specifiers
                        .iter()
                        .chain(requirement.specifiers().iter())
                        .cloned()
                        .collect();
                })
                .or_insert_with(|| requirement.specifiers().clone());
        }

        if let Some(members) = lock.member_group_metadata() {
            let group_root = self.group_root(groups);

            for (member, member_groups) in members {
                // The group root can contribute groups without being an install root.
                let is_install_root = self.roots().any(|root| root == member);
                if !is_install_root && group_root != Some(member) {
                    continue;
                }

                for (group, metadata) in member_groups {
                    if self.includes_group(Some(member), group, groups)
                        && let Some(requires_python) = &metadata.requires_python
                    {
                        group_requirements.insert(
                            RequiresPythonDeclaration::Member(member.clone(), Some(group.clone())),
                            requires_python.clone(),
                        );
                    }
                }
            }
        }

        for (group, metadata) in lock.workspace_group_metadata() {
            if self.includes_group(None, group, groups)
                && let Some(requires_python) = &metadata.requires_python
            {
                group_requirements.insert(
                    RequiresPythonDeclaration::Workspace(group.clone()),
                    requires_python.clone(),
                );
            }
        }

        let Some(requires_python) = RequiresPython::intersection(
            std::iter::once(lock.requires_python().specifiers()).chain(group_requirements.values()),
        ) else {
            return Err(EnvironmentError::DisjointLockedRequiresPython {
                locked: lock.requires_python().clone(),
                groups: group_requirements,
            });
        };
        Ok(ProjectPythonRequirement {
            requires_python,
            source: PythonRequirementSource::Lockfile {
                locked: lock.requires_python().clone(),
                groups: group_requirements,
            },
        })
    }

    /// Select installation roots from a project and its workspace.
    pub fn from_project(
        project: &'lock VirtualProject,
        lock: &'lock Lock,
        selection: PackageSelection<'lock>,
    ) -> Self {
        let workspace = project.workspace();
        match selection {
            PackageSelection::Projects([name]) => Self::Project {
                workspace,
                name,
                lock,
            },
            PackageSelection::Projects(names) => Self::Projects {
                workspace,
                names,
                lock,
            },
            PackageSelection::Workspace => {
                if let Some(project_name) = project.project_name() {
                    Self::Workspace {
                        workspace,
                        project_name: Some(project_name),
                        lock,
                    }
                } else {
                    Self::NonProjectWorkspace { workspace, lock }
                }
            }
            PackageSelection::NonProjectWorkspace => Self::NonProjectWorkspace { workspace, lock },
        }
    }

    /// Normalize project and lockfile selections before choosing installation roots.
    fn package_selection(&self) -> Option<PackageSelection<'lock>> {
        match self {
            Self::Project { name, .. } => {
                Some(PackageSelection::Projects(std::slice::from_ref(*name)))
            }
            Self::Projects { names, .. } => Some(PackageSelection::Projects(names)),
            Self::Workspace { .. } => Some(PackageSelection::Workspace),
            Self::NonProjectWorkspace { .. } => Some(PackageSelection::NonProjectWorkspace),
            Self::Lockfile { selection, .. } => Some(*selection),
            Self::Script { .. } => None,
        }
    }

    /// Return the project subject to workspace-root group precedence and its workspace, if available.
    fn selected_project(&self) -> Option<(&'lock PackageName, Option<&'lock Workspace>)> {
        match self {
            Self::Project {
                name, workspace, ..
            } => Some((name, Some(workspace))),
            Self::Lockfile {
                selection: PackageSelection::Projects([name]),
                ..
            } => Some((name, None)),
            Self::Projects { .. }
            | Self::Workspace { .. }
            | Self::NonProjectWorkspace { .. }
            | Self::Lockfile { .. }
            | Self::Script { .. } => None,
        }
    }

    /// Reject selected roots whose locked graph is unavailable on this Python version.
    pub(crate) fn validate_python(self, version: &Version) -> Result<(), EnvironmentError> {
        let marker = MarkerTree::expression(MarkerExpression::Version {
            key: MarkerValueVersion::PythonFullVersion,
            specifier: VersionSpecifier::equals_version(version.only_release()),
        });
        for name in self.roots() {
            if let Ok(Some(package)) = self.lock().find_by_name(name)
                && !package.is_included_by_marker(marker)
            {
                return Err(EnvironmentError::LockedRootPythonIncompatibility(
                    version.clone(),
                    name.clone(),
                ));
            }
        }
        Ok(())
    }

    /// Convert the target's locked packages to a [`Resolution`].
    pub fn to_resolution(
        self,
        marker_env: &ResolverMarkerEnvironment,
        tags: &Tags,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
        build_options: &BuildOptions,
        install_options: &InstallOptions,
    ) -> Result<Resolution, LockError> {
        // Package-backed project and workspace targets without conflicts can use concrete roots.
        // Other targets need the generic path to include manifest dependencies or evaluate
        // conflict markers from project roots.
        let use_concrete_roots = self.lock().conflicts().is_empty()
            && match self {
                Self::Project { workspace, .. }
                | Self::Projects { workspace, .. }
                | Self::Workspace { workspace, .. } => !workspace.is_non_project(),
                Self::Lockfile { lock, .. } => lock.root().is_some(),
                Self::NonProjectWorkspace { .. } | Self::Script { .. } => false,
            };
        if use_concrete_roots
            && self.group_root(groups).is_none()
            && let Some(roots) = self
                .roots()
                .map(|root_name| self.lock().find_by_name(root_name).ok().flatten())
                .collect::<Option<Vec<_>>>()
        {
            return self.lock().to_resolution(
                self.install_path(),
                roots,
                self.project_name(),
                marker_env,
                tags,
                extras,
                groups,
                build_options,
                install_options,
            );
        }

        Installable::to_resolution(
            &self,
            marker_env,
            tags,
            extras,
            groups,
            build_options,
            install_options,
        )
    }

    /// Return an iterator over the [`Index`] definitions in the target.
    pub(crate) fn indexes(self) -> impl Iterator<Item = &'lock Index> {
        match self {
            Self::Project { workspace, .. }
            | Self::Projects { workspace, .. }
            | Self::Workspace { workspace, .. }
            | Self::NonProjectWorkspace { workspace, .. } => {
                Either::Left(workspace.indexes().iter().chain(
                    workspace.packages().values().flat_map(|member| {
                        member
                            .pyproject_toml()
                            .tool
                            .as_ref()
                            .and_then(|tool| tool.uv.as_ref())
                            .and_then(|uv| uv.index.as_ref())
                            .into_iter()
                            .flatten()
                    }),
                ))
            }
            Self::Script { script, .. } => Either::Right(Either::Left(
                script
                    .metadata
                    .tool
                    .as_ref()
                    .and_then(|tool| tool.uv.as_ref())
                    .and_then(|uv| uv.top_level.index.as_deref())
                    .into_iter()
                    .flatten(),
            )),
            Self::Lockfile { .. } => Either::Right(Either::Right(std::iter::empty())),
        }
    }

    /// Return an iterator over all [`Sources`] defined by the target.
    pub(crate) fn sources(&self) -> impl Iterator<Item = &Source> {
        match self {
            Self::Project { workspace, .. }
            | Self::Projects { workspace, .. }
            | Self::Workspace { workspace, .. }
            | Self::NonProjectWorkspace { workspace, .. } => {
                Either::Left(workspace.sources().values().flat_map(Sources::iter).chain(
                    workspace.packages().values().flat_map(|member| {
                        member
                            .pyproject_toml()
                            .tool
                            .as_ref()
                            .and_then(|tool| tool.uv.as_ref())
                            .and_then(|uv| uv.sources.as_ref())
                            .map(ToolUvSources::inner)
                            .into_iter()
                            .flat_map(|sources| sources.values().flat_map(Sources::iter))
                    }),
                ))
            }
            Self::Script { script, .. } => Either::Right(Either::Left(
                script.sources().values().flat_map(Sources::iter),
            )),
            Self::Lockfile { .. } => Either::Right(Either::Right(std::iter::empty())),
        }
    }

    /// Return an iterator over all requirements defined by the target.
    pub(crate) fn requirements(
        &self,
    ) -> impl Iterator<Item = Cow<'lock, uv_pep508::Requirement<VerbatimParsedUrl>>> {
        match self {
            Self::Project { workspace, .. }
            | Self::Projects { workspace, .. }
            | Self::Workspace { workspace, .. }
            | Self::NonProjectWorkspace { workspace, .. } => {
                Either::Left(
                    // Iterate over the non-member requirements in the workspace.
                    workspace
                        .requirements()
                        .into_iter()
                        .map(Cow::Owned)
                        .chain(
                            workspace
                                .workspace_dependency_groups()
                                .ok()
                                .into_iter()
                                .flat_map(|dependency_groups| {
                                    dependency_groups
                                        .into_values()
                                        .flat_map(|group| group.requirements)
                                        .map(Cow::Owned)
                                }),
                        )
                        .chain(workspace.packages().values().flat_map(|member| {
                            // Iterate over all dependencies in each member.
                            let dependencies = member
                                .pyproject_toml()
                                .project
                                .as_ref()
                                .and_then(|project| project.dependencies.as_ref())
                                .into_iter()
                                .flatten();
                            let optional_dependencies = member
                                .pyproject_toml()
                                .project
                                .as_ref()
                                .and_then(|project| project.optional_dependencies.as_ref())
                                .into_iter()
                                .flat_map(|optional| optional.values())
                                .flatten();
                            let dependency_groups = member
                                .pyproject_toml()
                                .dependency_groups
                                .as_ref()
                                .into_iter()
                                .flatten()
                                .flat_map(|(_, dependencies)| {
                                    dependencies.iter().filter_map(|specifier| {
                                        if let DependencyGroupSpecifier::Requirement(requirement) =
                                            specifier
                                        {
                                            Some(requirement)
                                        } else {
                                            None
                                        }
                                    })
                                });
                            let dev_dependencies = member
                                .pyproject_toml()
                                .tool
                                .as_ref()
                                .and_then(|tool| tool.uv.as_ref())
                                .and_then(|uv| uv.dev_dependencies.as_ref())
                                .into_iter()
                                .flatten();
                            dependencies
                                .chain(optional_dependencies)
                                .chain(dependency_groups)
                                .filter_map(|requires_dist| {
                                    LenientRequirement::<VerbatimParsedUrl>::from_str(requires_dist)
                                        .map(uv_pep508::Requirement::from)
                                        .map(Cow::Owned)
                                        .ok()
                                })
                                .chain(dev_dependencies.map(Cow::Borrowed))
                        })),
                )
            }
            Self::Script { script, .. } => Either::Right(Either::Left(
                script
                    .metadata
                    .dependencies
                    .iter()
                    .flatten()
                    .map(Cow::Borrowed),
            )),
            Self::Lockfile { .. } => Either::Right(Either::Right(std::iter::empty())),
        }
    }

    pub(crate) fn build_constraints(&self) -> Constraints {
        self.lock().build_constraints(self.install_path())
    }

    /// Validate the extras requested by the [`ExtrasSpecification`].
    pub fn validate_extras(self, extras: &ExtrasSpecification) -> Result<(), EnvironmentError> {
        if extras.is_empty() {
            return Ok(());
        }
        match self {
            Self::Project { lock, .. }
            | Self::Projects { lock, .. }
            | Self::Workspace { lock, .. }
            | Self::NonProjectWorkspace { lock, .. }
            | Self::Lockfile { lock, .. } => {
                if !lock.supports_provides_extra() {
                    return Ok(());
                }

                let roots = self.roots().collect::<FxHashSet<_>>();
                // Read only the lockfile so frozen installs cannot select newly declared extras.
                let known_extras = lock
                    .packages()
                    .iter()
                    .filter(|package| roots.contains(package.name()))
                    .flat_map(|package| {
                        package
                            .provides_extras()
                            .iter()
                            .chain(package.optional_dependencies().keys())
                    })
                    .collect::<FxHashSet<_>>();

                for extra in extras.explicit_names() {
                    if !known_extras.contains(extra) {
                        return match self {
                            Self::Project { name, .. } => Err(
                                EnvironmentError::MissingExtraProject(extra.clone(), name.clone()),
                            ),
                            Self::Projects { .. } => {
                                Err(EnvironmentError::MissingExtraProjects(extra.clone()))
                            }
                            _ => Err(EnvironmentError::MissingExtraProjects(extra.clone())),
                        };
                    }
                }
            }
            Self::Script { .. } => {
                // We shouldn't get here if the list is empty so we can assume it isn't
                let extra = extras
                    .explicit_names()
                    .next()
                    .expect("non-empty extras")
                    .clone();
                return Err(EnvironmentError::MissingExtraScript(extra));
            }
        }

        Ok(())
    }

    /// Validate that selected non-root dependencies were resolved for the requested environment.
    pub fn validate_workspace_resolution(
        &self,
        extras: &ExtrasSpecification,
        groups: &DependencyGroupsWithDefaults,
        marker_environment: Option<&ResolverMarkerEnvironment>,
    ) -> Result<(), EnvironmentError> {
        if !groups.prod() && extras.is_empty() {
            return Ok(());
        }
        let lock = self.lock();
        let roots = self.roots().collect::<FxHashSet<_>>();
        let selected = lock
            .non_root_workspace_packages()
            .filter(|package| roots.contains(package.name()))
            .collect::<Vec<_>>();
        if selected.is_empty() {
            return Ok(());
        }
        let mut activated = roots
            .iter()
            .map(|name| ConflictItem::from((*name).clone()))
            .collect::<Vec<_>>();
        let group_root = self.group_root(groups);
        for package in lock.workspace_packages() {
            if roots.contains(package.name()) {
                for extra in extras.extra_names(
                    package
                        .provides_extras()
                        .iter()
                        .chain(package.optional_dependencies().keys()),
                ) {
                    activated.push(ConflictItem::from((package.name().clone(), extra.clone())));
                }
            }
            if roots.contains(package.name()) || group_root == Some(package.name()) {
                for group in package
                    .dependency_groups()
                    .keys()
                    .chain(package.resolved_dependency_groups().keys())
                {
                    if self.includes_group(Some(package.name()), group, groups) {
                        activated.push(ConflictItem::from((package.name().clone(), group.clone())));
                    }
                }
            }
        }
        let resolved = lock.resolved_workspace_reachability(self.install_path(), &activated)?;
        let domain = implicit_constraints_marker(
            self.python_requirement(groups)?
                .requires_python
                .to_marker_tree(),
            lock.supported_environments(),
        );
        let available = |marker: MarkerTree| {
            if let Some(environment) = marker_environment {
                marker.evaluate(environment.markers(), &[])
            } else {
                domain.and(marker.negate()).is_false()
            }
        };
        for package in selected {
            if groups.prod() {
                let marker = resolved
                    .get(&(package.name(), None))
                    .copied()
                    .unwrap_or(MarkerTree::FALSE);
                if !available(marker) {
                    return Err(EnvironmentError::UnresolvedWorkspacePackage(
                        package.name().clone(),
                    ));
                }
            }
            for extra in extras.extra_names(
                package
                    .provides_extras()
                    .iter()
                    .chain(package.optional_dependencies().keys()),
            ) {
                let marker = resolved
                    .get(&(package.name(), Some(extra)))
                    .copied()
                    .unwrap_or(MarkerTree::FALSE);
                if !available(marker) {
                    return Err(EnvironmentError::UnresolvedWorkspaceExtra {
                        package: package.name().clone(),
                        extra: extra.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validate the dependency groups requested by the [`DependencyGroupSpecifier`].
    pub fn validate_groups(
        self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<(), EnvironmentError> {
        let lock = self.lock();
        let roots = self
            .roots()
            .chain(self.group_root(groups))
            .collect::<FxHashSet<_>>();
        for package in lock
            .non_root_workspace_packages()
            .filter(|package| roots.contains(package.name()))
        {
            for group in package
                .dependency_groups()
                .keys()
                .chain(package.resolved_dependency_groups().keys())
            {
                if self.includes_group(Some(package.name()), group, groups) {
                    return Err(EnvironmentError::UnresolvedWorkspaceGroup {
                        package: package.name().clone(),
                        group: group.clone(),
                    });
                }
            }
        }

        // If no groups were specified, short-circuit.
        if groups.explicit_names().next().is_none() {
            return Ok(());
        }

        match self {
            Self::Lockfile {
                lock, selection, ..
            } => {
                // Only a single selected project inherits groups from the workspace root.
                let workspace_root = self
                    .selected_project()
                    .and_then(|_| lock.root())
                    .map(Package::name);
                let roots = self.roots().chain(workspace_root).collect::<FxHashSet<_>>();
                let known_groups = lock
                    .packages()
                    .iter()
                    .filter(|package| roots.contains(package.name()))
                    .flat_map(|package| {
                        package
                            .dependency_groups()
                            .keys()
                            .chain(package.resolved_dependency_groups().keys())
                    })
                    .chain(lock.dependency_groups().keys())
                    .collect::<FxHashSet<_>>();
                for group in groups.explicit_names() {
                    if !known_groups.contains(group) {
                        return match selection {
                            PackageSelection::Projects([_]) => {
                                Err(EnvironmentError::MissingGroupProject(group.clone()))
                            }
                            PackageSelection::Projects(_)
                            | PackageSelection::Workspace
                            | PackageSelection::NonProjectWorkspace => {
                                Err(EnvironmentError::MissingGroupProjects(group.clone()))
                            }
                        };
                    }
                }
            }
            Self::Project {
                lock, workspace, ..
            }
            | Self::Projects {
                lock, workspace, ..
            }
            | Self::Workspace {
                lock, workspace, ..
            }
            | Self::NonProjectWorkspace { lock, workspace } => {
                // Validate inherited root groups even when `--no-group` excludes them from
                // installation and therefore omits the root from the selected group roots.
                let workspace_root = matches!(self, Self::Project { .. })
                    .then(|| lock.root())
                    .flatten()
                    .map(Package::name);
                let roots = self.roots().chain(workspace_root).collect::<FxHashSet<_>>();
                let member_groups = lock
                    .packages()
                    .iter()
                    .filter(|package| roots.contains(package.name()))
                    .flat_map(|package| {
                        // Reject groups added to the workspace after the lock was written.
                        package
                            .dependency_groups()
                            .keys()
                            .chain(package.resolved_dependency_groups().keys())
                            .map(Cow::Borrowed)
                    });

                // Groups defined directly on a non-project workspace root are not members.
                let workspace_groups = workspace
                    .is_non_project()
                    .then(|| workspace.workspace_dependency_groups().ok())
                    .flatten()
                    .into_iter()
                    .flat_map(|dependency_groups| dependency_groups.into_keys().map(Cow::Owned));

                let known_groups = member_groups
                    .chain(workspace_groups)
                    .collect::<FxHashSet<_>>();

                for group in groups.explicit_names() {
                    if !known_groups.contains(group) {
                        return match self {
                            Self::Project { .. } => {
                                Err(EnvironmentError::MissingGroupProject(group.clone()))
                            }
                            _ => Err(EnvironmentError::MissingGroupProjects(group.clone())),
                        };
                    }
                }
            }
            Self::Script { .. } => {
                if let Some(group) = groups.explicit_names().next() {
                    return Err(EnvironmentError::MissingGroupScript(group.clone()));
                }
            }
        }

        Ok(())
    }

    /// Returns the names of all packages in the workspace that will be installed.
    ///
    /// Note this only includes workspace members.
    pub(super) fn packages(
        &self,
        extras: &ExtrasSpecification,
        groups: &DependencyGroupsWithDefaults,
    ) -> BTreeSet<&PackageName> {
        match self.package_selection() {
            Some(PackageSelection::Projects(_)) => {
                let lock = self.lock();
                let roots = self.roots().collect::<FxHashSet<_>>();

                // Collect the packages by name for efficient lookup.
                let packages = lock
                    .packages()
                    .iter()
                    .map(|package| (package.name(), package))
                    .collect::<BTreeMap<_, _>>();

                // We'll include all specified projects
                let mut required_members = BTreeSet::new();
                for name in &roots {
                    required_members.insert(*name);
                }

                // Find all workspace member dependencies recursively for all specified packages
                let mut queue: VecDeque<(&PackageName, Option<&ExtraName>)> = VecDeque::new();
                let mut seen: FxHashSet<(&PackageName, Option<&ExtraName>)> = FxHashSet::default();

                for (name, root_kind) in roots
                    .iter()
                    .copied()
                    .map(|name| (name, InstallableRootKind::Production))
                    .chain(
                        self.group_root(groups)
                            .map(|name| (name, InstallableRootKind::DependencyGroups)),
                    )
                {
                    let Some(root_package) = packages.get(name) else {
                        continue;
                    };

                    if root_kind == InstallableRootKind::Production && groups.prod() {
                        // Add the root package
                        if seen.insert((name, None)) {
                            queue.push_back((name, None));
                        }

                        // Add explicitly activated extras for the root package
                        for extra in extras.extra_names(root_package.optional_dependencies().keys())
                        {
                            if seen.insert((name, Some(extra))) {
                                queue.push_back((name, Some(extra)));
                            }
                        }
                    }

                    // Add activated dependency groups for the root package
                    for (group_name, dependencies) in root_package.resolved_dependency_groups() {
                        if !self.includes_group(Some(root_package.name()), group_name, groups) {
                            continue;
                        }
                        for dependency in dependencies {
                            let dep_name = dependency.package_name();
                            if seen.insert((dep_name, None)) {
                                queue.push_back((dep_name, None));
                            }
                            for extra in dependency.extra() {
                                if seen.insert((dep_name, Some(extra))) {
                                    queue.push_back((dep_name, Some(extra)));
                                }
                            }
                        }
                    }
                }

                while let Some((package_name, extra)) = queue.pop_front() {
                    if lock.workspace_members().contains(package_name) {
                        required_members.insert(package_name);
                    }

                    let Some(package) = packages.get(package_name) else {
                        continue;
                    };

                    let Some(dependencies) = extra
                        .map(|extra_name| {
                            package
                                .optional_dependencies()
                                .get(extra_name)
                                .map(Vec::as_slice)
                        })
                        .unwrap_or(Some(package.dependencies()))
                    else {
                        continue;
                    };

                    for dependency in dependencies {
                        let name = dependency.package_name();
                        if seen.insert((name, None)) {
                            queue.push_back((name, None));
                        }
                        for extra in dependency.extra() {
                            if seen.insert((name, Some(extra))) {
                                queue.push_back((name, Some(extra)));
                            }
                        }
                    }
                }

                required_members
            }
            Some(PackageSelection::Workspace | PackageSelection::NonProjectWorkspace) => {
                // Return all workspace members
                self.lock()
                    .workspace_member_paths()
                    .map(|(name, _)| name)
                    .collect()
            }
            None => {
                // Scripts don't have workspace members
                BTreeSet::new()
            }
        }
    }
}
