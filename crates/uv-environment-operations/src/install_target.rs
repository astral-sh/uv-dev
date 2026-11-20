use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::str::FromStr;

use itertools::{Either, Itertools};
use rustc_hash::FxHashSet;

use uv_configuration::{
    BuildOptions, Constraints, DependencyGroupsWithDefaults, ExtrasSpecification,
    ExtrasSpecificationWithDefaults, InstallOptions, InstallTarget as InstallOptionTarget,
    TargetTriple,
};
use uv_distribution_types::{
    Dist, Edge, Index, Name, Node, RequiresPython, Resolution, ResolvedDist,
};
use uv_lock::{
    DependencyScope, DependencySelection, Installable, InstallableRootKind, Lock, LockError,
    Package,
};
use uv_normalize::{DEV_DEPENDENCIES, ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;
use uv_platform_tags::Tags;
use uv_pypi_types::{
    DependencyGroupSpecifier, DependencyGroups, LenientRequirement, ResolverMarkerEnvironment,
    VerbatimParsedUrl,
};
use uv_python_discovery::ProjectPythonRequirement;
use uv_python_discovery::PythonRequirementSource;
use uv_python_interpreter::Interpreter;
use uv_resolve_operations::{resolution_markers, resolution_tags};
use uv_scripts::Pep723Script;
use uv_workspace::dependency_groups::FlatDependencyGroups;
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

/// The packages requested for the project environment.
pub struct SyncSelection<'a> {
    pub extras: &'a ExtrasSpecificationWithDefaults,
    pub groups: &'a DependencyGroupsWithDefaults,
    pub install_options: &'a InstallOptions,
    pub build_options: &'a BuildOptions,
}

/// Where a project can obtain a tool declared in its lockfile.
pub enum ToolLocation {
    /// No declaration applies to the project interpreter.
    Inactive,
    /// Synchronization includes the preferred declaration.
    Project,
    /// Re-evaluate the declarations against the interpreter for the cached environment.
    Cache,
}

/// An installation target paired with a Python version and platform supported by its lockfile.
pub struct LockInstallation<'lock, 'env> {
    target: InstallTarget<'lock>,
    markers: ResolverMarkerEnvironment,
    tags: Cow<'env, Tags>,
}

impl<'lock> LockInstallation<'lock, '_> {
    pub(crate) fn tags(&self) -> &Tags {
        &self.tags
    }

    /// Select direct dependencies that apply to this installation.
    pub fn dependency_selection(
        &self,
        project_name: Option<&PackageName>,
        dependency_name: &PackageName,
    ) -> Result<DependencySelection<'lock>, EnvironmentError> {
        Ok(self
            .target
            .lock()
            .dependency_selection(project_name, dependency_name, self.markers.markers())
            .map_err(anyhow::Error::msg)?)
    }

    /// Locate the first applicable declaration in scope order for this project sync.
    pub fn tool_location(
        &self,
        owner: Option<&PackageName>,
        name: &PackageName,
        scopes: &[DependencyScope<'_>],
        sync: &SyncSelection<'_>,
    ) -> Result<ToolLocation, EnvironmentError> {
        let selection = self.dependency_selection(owner, name)?;
        let Some(scope) = scopes
            .iter()
            .copied()
            .find(|scope| selection.in_scope(*scope).is_some())
        else {
            return Ok(ToolLocation::Inactive);
        };
        let is_root = owner.is_none_or(|name| self.target.roots().any(|root| root == name));
        let included = match scope {
            DependencyScope::Group(group) => {
                (is_root || self.target.group_root(sync.groups) == owner)
                    && self.target.includes_group(owner, group, sync.groups)
            }
            DependencyScope::Production if is_root => sync.groups.prod(),
            DependencyScope::Production => {
                // A selected member can pull in the declaring package transitively.
                let resolution = self.to_resolution(
                    sync.extras,
                    sync.groups,
                    sync.build_options,
                    sync.install_options,
                )?;
                let graph = resolution.graph();
                graph.raw_edges().iter().any(|edge| {
                    matches!(edge.weight, Edge::Prod)
                        && matches!(&graph[edge.source()], Node::Dist { dist, .. } if Some(dist.name()) == owner)
                        && matches!(&graph[edge.target()], Node::Dist { dist, install: true, .. } if dist.name() == name)
                })
            }
        };
        Ok(if included {
            ToolLocation::Project
        } else {
            ToolLocation::Cache
        })
    }

    /// Select a tool and materialize its subgraph for the same environment.
    pub(crate) fn resolve_tool(
        &self,
        project_name: Option<&PackageName>,
        name: &PackageName,
        scope: DependencyScope<'_>,
        build_options: &BuildOptions,
    ) -> Result<Option<Resolution>, EnvironmentError> {
        let selection = self.dependency_selection(project_name, name)?;
        let Some(dependency) = selection.in_scope(scope) else {
            return Ok(None);
        };
        let resolution = self.target.lock().to_resolution_from_dependency(
            self.target.install_path(),
            dependency,
            project_name,
            &self.markers,
            &self.tags,
            build_options,
            &InstallOptions::default(),
        )?;
        Ok(Some(exclude_virtual_projects(resolution)))
    }

    /// Materialize the requested packages, extras, and dependency groups.
    pub(crate) fn to_resolution(
        &self,
        extras: &ExtrasSpecificationWithDefaults,
        groups: &DependencyGroupsWithDefaults,
        build_options: &BuildOptions,
        install_options: &InstallOptions,
    ) -> Result<Resolution, LockError> {
        self.target
            .to_resolution(
                &self.markers,
                &self.tags,
                extras,
                groups,
                build_options,
                install_options,
            )
            .map(exclude_virtual_projects)
    }
}

/// Virtual projects contribute dependencies to the resolution but have no distribution to install.
fn exclude_virtual_projects(resolution: Resolution) -> Resolution {
    resolution.filter(|dist| {
        let ResolvedDist::Installable { dist, .. } = dist else {
            return true;
        };

        let Dist::Source(dist) = dist.as_ref() else {
            return true;
        };

        !dist.is_virtual()
    })
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
    /// Intersect the lockfile's Python requirement with the selected groups' requirements.
    pub fn python_requirement(
        &self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<ProjectPythonRequirement, EnvironmentError> {
        let lock = self.lock();
        let mut group_requirements = RequiresPythonSources::new();

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

        Self::intersect_python_requirements(lock, group_requirements)
    }

    /// Intersect the workspace lock's Python requirement with the bound on one root group.
    /// A checker inherited from the root runs independently of the members' groups.
    pub fn root_group_python_requirement(
        workspace: &Workspace,
        lock: &Lock,
        group: &GroupName,
    ) -> Result<ProjectPythonRequirement, EnvironmentError> {
        let root = lock.root().map(Package::name);
        let bound = if let Some(mut members) = lock.member_group_metadata() {
            match root {
                Some(name) => members
                    .find(|(member, _)| *member == name)
                    .and_then(|(_, groups)| groups.get(group)),
                None => lock.workspace_group_metadata().get(group),
            }
            .and_then(|metadata| metadata.requires_python.clone())
        } else {
            // Earlier locks encode the bound in the edge markers. The manifest also exposes
            // it as a requirement we can use to find Python before evaluating those markers.
            FlatDependencyGroups::from_group(
                workspace.install_path(),
                workspace.pyproject_toml(),
                group,
            )?
            .and_then(|group| group.requires_python)
        };
        let mut requirements = RequiresPythonSources::new();
        if let Some(bound) = bound {
            let declaration = match root {
                Some(name) => RequiresPythonDeclaration::Member(name.clone(), Some(group.clone())),
                None => RequiresPythonDeclaration::Workspace(group.clone()),
            };
            requirements.insert(declaration, bound);
        }
        Self::intersect_python_requirements(lock, requirements)
    }

    fn intersect_python_requirements(
        lock: &Lock,
        group_requirements: RequiresPythonSources,
    ) -> Result<ProjectPythonRequirement, EnvironmentError> {
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
            _ => None,
        }
    }

    /// Prepare to install from the lockfile for an interpreter and optional platform override.
    pub fn for_environment<'env>(
        self,
        interpreter: &'env Interpreter,
        python_platform: Option<&TargetTriple>,
    ) -> Result<LockInstallation<'lock, 'env>, EnvironmentError> {
        let lock = self.lock();
        if !lock
            .requires_python()
            .contains(interpreter.python_version())
        {
            return Err(EnvironmentError::LockedPythonIncompatibility(
                interpreter.python_version().clone(),
                lock.requires_python().clone(),
            ));
        }
        let markers = resolution_markers(None, python_platform, interpreter);
        let environments = lock.supported_environments();
        if !environments.is_empty() && !environments.iter().any(|env| env.evaluate(&markers, &[])) {
            // The simplified markers reflect what the user wrote, without implicit Python bounds.
            return Err(EnvironmentError::LockedPlatformIncompatibility(
                lock.simplified_supported_environments()
                    .into_iter()
                    .filter_map(MarkerTree::contents)
                    .map(|env| format!("`{env}`"))
                    .join(", "),
            ));
        }

        let tags = resolution_tags(None, python_platform, interpreter)?;
        Ok(LockInstallation {
            target: self,
            markers,
            tags,
        })
    }

    /// Project the lockfile onto an arbitrary environment, e.g., to identify installed packages.
    /// For installation, use [`Self::for_environment`].
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
                script.metadata.dependencies.iter().map(Cow::Borrowed),
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

    /// Validate the dependency groups requested by the [`DependencyGroupSpecifier`].
    pub fn validate_groups(
        self,
        groups: &DependencyGroupsWithDefaults,
    ) -> Result<(), EnvironmentError> {
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
                    if lock.members().contains(package_name) {
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
                self.lock().members().iter().collect()
            }
            None => {
                // Scripts don't have workspace members
                BTreeSet::new()
            }
        }
    }
}
