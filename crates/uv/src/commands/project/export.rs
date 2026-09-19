use std::env;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use itertools::Itertools;
use owo_colors::OwoColorize;
use rustc_hash::FxHashSet;
use serde::Deserialize;

use uv_cache::Cache;
use uv_client::{BaseClientBuilder, RegistryClientBuilder};
use uv_configuration::{
    ActiveEnvironment, Concurrency, DependencyGroups, DependencyGroupsWithDefaults, EditableMode,
    ExportFormat, ExtrasSpecification, ExtrasSpecificationWithDefaults, InstallOptions,
};
use uv_distribution_types::Verbatim;
use uv_lock::{
    ExportableRequirements, Installable, Lock, PylockToml, RequirementsTxtExport, cyclonedx_json,
};
use uv_normalize::{DefaultExtras, DefaultGroups, ExtraName, GroupName, PackageName};
use uv_pep508::MarkerTree;
use uv_preview::{Preview, PreviewFeature};
use uv_python::{ConfigDiscovery, PythonDownloads, PythonPreference, PythonRequest};
use uv_requirements::is_pylock_toml;
use uv_scripts::Pep723Script;
use uv_settings::PythonInstallMirrors;
use uv_warnings::warn_user;
use uv_workspace::{DiscoveryOptions, MemberDiscovery, VirtualProject, Workspace, WorkspaceCache};

use crate::commands::pip::loggers::DefaultResolveLogger;
use crate::commands::project::install_target::InstallTarget;
use crate::commands::project::lock::{LockMode, LockOperation};
use crate::commands::project::lock_target::LockTarget;
use crate::commands::project::{
    ProjectEnvironmentPolicy, ProjectInterpreter, ScriptInterpreter, UniversalState,
    WorkspacePython, detect_conflicts,
};
use crate::commands::{ExitStatus, OutputWriter, UvError};
use crate::printer::Printer;
use crate::settings::{FrozenSource, LockCheck, ResolverSettings};

#[derive(Debug, Clone)]
#[expect(clippy::large_enum_variant)]
enum ExportTarget {
    /// A PEP 723 script, with inline metadata.
    Script(Pep723Script),

    /// A project with a `pyproject.toml`.
    Project(VirtualProject),
}

impl<'lock> From<&'lock ExportTarget> for LockTarget<'lock> {
    fn from(value: &'lock ExportTarget) -> Self {
        match value {
            ExportTarget::Script(script) => Self::Script(script),
            ExportTarget::Project(project) => Self::Workspace(project.workspace()),
        }
    }
}

/// Independent selections and destinations for a batch export.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportBatch {
    export: Vec<BatchExport>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct BatchExport {
    output_file: PathBuf,
    #[serde(default)]
    package: Vec<PackageName>,
    #[serde(default)]
    resolution_root: Vec<PackageName>,
    #[serde(default)]
    all_packages: bool,
    #[serde(default)]
    extra: Vec<ExtraName>,
    #[serde(default)]
    no_extra: Vec<ExtraName>,
    #[serde(default)]
    all_extras: bool,
    #[serde(default)]
    group: Vec<GroupName>,
    #[serde(default)]
    no_group: Vec<GroupName>,
    #[serde(default)]
    only_group: Vec<GroupName>,
    #[serde(default)]
    all_groups: bool,
    #[serde(default)]
    no_default_groups: bool,
}

impl ExportBatch {
    /// Read and validate the manifest, resolving output paths relative to its directory.
    async fn read(path: &Path) -> Result<Self> {
        let contents = fs_err::tokio::read_to_string(path).await?;
        let mut batch: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse export manifest `{}`", path.display()))?;
        if batch.export.is_empty() {
            bail!("Export manifest must contain at least one `[[export]]` entry");
        }
        let parent = path.parent().unwrap_or(Path::new("."));
        let mut outputs = FxHashSet::default();
        for entry in &mut batch.export {
            entry.output_file = uv_fs::normalize_absolute_path(&std::path::absolute(
                parent.join(&entry.output_file),
            )?)?;
            if !outputs.insert(entry.output_file.clone()) {
                bail!("Duplicate export output: `{}`", entry.output_file.display());
            }
            if entry.all_packages && !entry.package.is_empty() {
                bail!("`all-packages` cannot be combined with `package`");
            }
            if entry.all_packages && !entry.resolution_root.is_empty() {
                bail!("`all-packages` cannot be combined with `resolution-root`");
            }
            if entry.all_extras && !entry.extra.is_empty() {
                bail!("`all-extras` cannot be combined with `extra`");
            }
            if !entry.only_group.is_empty() && (!entry.extra.is_empty() || entry.all_extras) {
                bail!("`only-group` cannot be combined with `extra` or `all-extras`");
            }
            if !entry.only_group.is_empty() && (!entry.group.is_empty() || entry.all_groups) {
                bail!("`only-group` cannot be combined with `group` or `all-groups`");
            }
        }
        Ok(batch)
    }
}

/// Export the project's `uv.lock` in an alternate format.
#[expect(clippy::fn_params_excessive_bools)]
pub(crate) async fn export(
    project_dir: &Path,
    format: Option<ExportFormat>,
    all_packages: bool,
    package: Vec<PackageName>,
    resolution_root: Vec<PackageName>,
    prune: Vec<PackageName>,
    hashes: bool,
    install_options: InstallOptions,
    output_file: Option<PathBuf>,
    batch: Option<PathBuf>,
    extras: ExtrasSpecification,
    groups: DependencyGroups,
    editable: Option<EditableMode>,
    lock_check: LockCheck,
    frozen: Option<FrozenSource>,
    include_annotations: bool,
    include_header: bool,
    include_index_url: bool,
    include_find_links: bool,
    script: Option<Pep723Script>,
    python: Option<String>,
    install_mirrors: PythonInstallMirrors,
    settings: ResolverSettings,
    client_builder: BaseClientBuilder<'_>,
    python_preference: PythonPreference,
    python_downloads: PythonDownloads,
    concurrency: Concurrency,
    config_discovery: ConfigDiscovery,
    quiet: bool,
    cache: &Cache,
    workspace_cache: &WorkspaceCache,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    let batch = if let Some(path) = batch {
        if !preview.is_enabled(PreviewFeature::BatchExport) {
            warn_user!(
                "`uv export --batch` is experimental and may change without warning. Pass `--preview-features {}` to disable this warning.",
                PreviewFeature::BatchExport
            );
        }
        Some(ExportBatch::read(&path).await?)
    } else {
        None
    };

    // Identify the target.
    let target = if let Some(script) = script {
        ExportTarget::Script(script)
    } else {
        let project = if frozen.is_some() {
            let options = DiscoveryOptions {
                members: if package.is_empty()
                    && batch.as_ref().is_none_or(|batch| {
                        batch.export.iter().all(|entry| entry.package.is_empty())
                    }) {
                    MemberDiscovery::None
                } else {
                    MemberDiscovery::Existing
                },
                ..DiscoveryOptions::default()
            };

            if let [name] = package.as_slice() {
                VirtualProject::discover_with_package(
                    project_dir,
                    &options,
                    cache,
                    workspace_cache,
                    name.clone(),
                )
                .await?
            } else {
                VirtualProject::discover(project_dir, &options, cache, workspace_cache).await?
            }
        } else if let [name] = package.as_slice() {
            VirtualProject::discover_with_package(
                project_dir,
                &DiscoveryOptions::default(),
                cache,
                workspace_cache,
                name.clone(),
            )
            .await?
        } else {
            let project = VirtualProject::discover(
                project_dir,
                &DiscoveryOptions::default(),
                cache,
                workspace_cache,
            )
            .await?;

            for name in &package {
                if !project.workspace().packages().contains_key(name) {
                    return Err(anyhow::anyhow!("Package `{name}` not found in workspace"));
                }
            }

            project
        };
        ExportTarget::Project(project)
    };

    // Find an interpreter for the project, unless `--frozen` is set.
    let interpreter = if frozen.is_some() {
        None
    } else {
        Some(match &target {
            ExportTarget::Script(script) => ScriptInterpreter::discover(
                script.into(),
                python.as_deref().map(PythonRequest::parse),
                &client_builder,
                python_preference,
                python_downloads,
                &install_mirrors,
                false,
                config_discovery,
                ActiveEnvironment::Ignore,
                cache,
                printer,
            )
            .await?
            .into_interpreter(),
            ExportTarget::Project(project) => {
                // Selected groups can impose additional Python requirements on a single export.
                // Batch entries may have incompatible group requirements, so choose the interpreter
                // for locking using only workspace requirements, as `uv lock` does. Each output's
                // groups and project defaults are applied separately when rendering below.
                let interpreter_groups = if batch.is_some() {
                    DependencyGroupsWithDefaults::none()
                } else {
                    groups.with_defaults(project.default_groups()?)
                };
                let workspace_python = WorkspacePython::from_request(
                    python.as_deref().map(PythonRequest::parse),
                    Some(project.workspace()),
                    &interpreter_groups,
                    project_dir,
                    config_discovery,
                )
                .await?;
                ProjectInterpreter::discover(
                    project.workspace(),
                    &interpreter_groups,
                    workspace_python,
                    &client_builder,
                    python_preference,
                    python_downloads,
                    &install_mirrors,
                    ProjectEnvironmentPolicy::Optional,
                    ActiveEnvironment::Ignore,
                    cache,
                    printer,
                )
                .await?
                .into_interpreter()
            }
        })
    };

    // Determine the lock mode.
    let mode = if let Some(frozen_source) = frozen {
        LockMode::Frozen(frozen_source.into())
    } else if let LockCheck::Enabled(lock_check) = lock_check {
        LockMode::Locked(interpreter.as_ref().unwrap(), lock_check)
    } else if matches!(target, ExportTarget::Script(_))
        && !LockTarget::from(&target).lock_path().is_file()
    {
        // If we're locking a script, avoid creating a lockfile if it doesn't already exist.
        LockMode::DryRun(interpreter.as_ref().unwrap())
    } else {
        LockMode::Write(interpreter.as_ref().unwrap())
    };

    // Initialize any shared state.
    let state = UniversalState::default();

    // Lock the project.
    let lock = match Box::pin(
        LockOperation::new(
            mode,
            &settings,
            &client_builder,
            &state,
            Box::new(DefaultResolveLogger),
            &concurrency,
            cache,
            workspace_cache,
            printer,
            preview,
        )
        .execute((&target).into()),
    )
    .await
    {
        Ok(result) => result.into_lock(),
        Err(err) => return Err(UvError::from(err).into()),
    };

    if let Some(batch) = &batch {
        let ExportTarget::Project(project) = &target else {
            bail!("`--batch` does not support scripts");
        };
        let mut writers = Vec::with_capacity(batch.export.len());
        for entry in &batch.export {
            let default_groups = project.default_groups_for_packages(&entry.package)?;
            let groups = DependencyGroups::from_args(
                None,
                entry.group.clone(),
                entry.no_group.clone(),
                entry.no_default_groups,
                entry.only_group.clone(),
                entry.all_groups,
            )
            .with_defaults(default_groups);
            let extras = ExtrasSpecification::from_args(
                entry.extra.clone(),
                entry.no_extra.clone(),
                false,
                vec![],
                entry.all_extras,
            )
            .with_defaults(DefaultExtras::default());
            writers.push(
                render_export(
                    &target,
                    &lock,
                    format,
                    entry.all_packages,
                    &entry.package,
                    &entry.resolution_root,
                    &prune,
                    hashes,
                    &install_options,
                    Some(&entry.output_file),
                    &extras,
                    &groups,
                    editable.clone(),
                    include_annotations,
                    include_header,
                    include_index_url,
                    include_find_links,
                    &settings,
                    &client_builder,
                    &concurrency,
                    true,
                    cache,
                    preview,
                )
                .await
                .with_context(|| format!("Failed to export `{}`", entry.output_file.display()))?,
            );
        }
        // Render every selection before replacing any output, so invalid selections leave files intact.
        for writer in writers {
            writer.commit().await?;
        }
        return Ok(ExitStatus::Success);
    }

    let default_groups = match &target {
        ExportTarget::Project(project) => project.default_groups()?,
        ExportTarget::Script(_) => DefaultGroups::default(),
    };
    let groups = groups.with_defaults(default_groups);
    let extras = extras.with_defaults(DefaultExtras::default());

    render_export(
        &target,
        &lock,
        format,
        all_packages,
        &package,
        &resolution_root,
        &prune,
        hashes,
        &install_options,
        output_file.as_deref(),
        &extras,
        &groups,
        editable,
        include_annotations,
        include_header,
        include_index_url,
        include_find_links,
        &settings,
        &client_builder,
        &concurrency,
        quiet,
        cache,
        preview,
    )
    .await?
    .commit()
    .await?;

    Ok(ExitStatus::Success)
}

/// An export has independent traversal roots and resolution-context roots.
struct ContextualExportTarget<'context, 'lock> {
    target: InstallTarget<'lock>,
    context: Option<&'context [PackageName]>,
}

impl<'lock> Installable<'lock> for ContextualExportTarget<'_, 'lock> {
    fn install_path(&self) -> &'lock Path {
        self.target.install_path()
    }

    fn lock(&self) -> &'lock Lock {
        self.target.lock()
    }

    fn roots(&self) -> impl Iterator<Item = &PackageName> {
        self.target.roots()
    }

    fn export_context(&self) -> Option<&[PackageName]> {
        self.context
    }

    fn group_root(&self, groups: &DependencyGroupsWithDefaults) -> Option<&PackageName> {
        self.target.group_root(groups)
    }

    fn includes_group(
        &self,
        package: Option<&PackageName>,
        group: &GroupName,
        groups: &DependencyGroupsWithDefaults,
    ) -> bool {
        self.target.includes_group(package, group, groups)
    }

    fn project_name(&self) -> Option<&PackageName> {
        self.target.project_name()
    }
}

/// Select a locked root context without changing which packages are emitted.
fn select_export_context(
    workspace: &Workspace,
    target: &InstallTarget<'_>,
    explicit: &[PackageName],
    extras: &ExtrasSpecificationWithDefaults,
    groups: &DependencyGroupsWithDefaults,
) -> Result<Option<Vec<PackageName>>> {
    if workspace.resolution_roots().is_none() {
        if !explicit.is_empty() {
            bail!("`--resolution-root` requires an explicit-root workspace");
        }
        return Ok(None);
    }

    let lock = target.lock();
    let selected = target.roots().cloned().collect::<Vec<_>>();
    if explicit.is_empty() && selected.iter().all(|name| lock.members().contains(name)) {
        return Ok(None);
    }
    // Context inference applies only to packages already present in the lock. The ordinary
    // exporter reports missing or ambiguous package entries with its existing diagnostics.
    if explicit.is_empty()
        && selected.iter().any(|name| match lock.find_by_name(name) {
            Ok(Some(_)) => false,
            Ok(None) | Err(_) => true,
        })
    {
        return Ok(None);
    }

    let production_extras = ExtrasSpecification::default().with_defaults(DefaultExtras::default());
    let production_groups = DependencyGroups::default().with_defaults(DefaultGroups::default());
    let install_options = InstallOptions::default();

    let context = if explicit.is_empty() {
        let [name] = selected.as_slice() else {
            bail!(
                "Exporting several non-root members requires an explicit `--resolution-root` selection"
            );
        };
        let mut candidates = Vec::new();
        let mut first_projection = None;
        let mut ambiguous = false;
        for root in lock.members() {
            let root_target = InstallTarget::Project {
                workspace,
                name: root,
                lock,
            };
            let closure = ExportableRequirements::from_lock(
                &root_target,
                &[],
                &production_extras,
                &production_groups,
                false,
                &install_options,
            )?;
            if !closure.contains_package(name) {
                continue;
            }
            let roots = std::slice::from_ref(root);
            let projection_target = ContextualExportTarget {
                target: *target,
                context: Some(roots),
            };
            let projection = ExportableRequirements::from_lock(
                &projection_target,
                &[],
                extras,
                groups,
                false,
                &install_options,
            )?;
            if let Some(first) = &first_projection {
                if !projection.same_packages(first) {
                    ambiguous = true;
                }
            } else {
                first_projection = Some(projection);
            }
            candidates.push(root.clone());
        }
        if ambiguous {
            bail!(
                "Package `{name}` has different dependency closures in workspace roots {}; select one with `--resolution-root`",
                candidates.iter().map(|name| format!("`{name}`")).join(", ")
            );
        }
        let Some(root) = candidates.into_iter().next() else {
            bail!("Package `{name}` is not reachable from a locked workspace root");
        };
        vec![root]
    } else {
        let mut roots = explicit.to_vec();
        roots.sort();
        roots.dedup();
        for root in &roots {
            if !lock.members().contains(root) {
                bail!("Package `{root}` is not a locked workspace resolution root");
            }
        }
        roots
    };

    let context_target = InstallTarget::Projects {
        workspace,
        names: &context,
        lock,
    };
    detect_conflicts(&context_target, &production_extras, &production_groups)?;
    let mut environment = MarkerTree::TRUE;
    for root in &context {
        let package = lock
            .find_by_name(root)
            .map_err(anyhow::Error::msg)?
            .with_context(|| {
                format!("Workspace resolution root `{root}` is missing from the lockfile")
            })?;
        if !package.fork_markers().is_empty() {
            environment = environment.and(
                package
                    .fork_markers()
                    .iter()
                    .fold(MarkerTree::FALSE, |marker, fork| marker.or(fork.pep508())),
            );
        }
    }
    if environment.is_false() {
        bail!("The selected resolution roots have no common supported environment");
    }
    let context_target = ContextualExportTarget {
        target: context_target,
        context: Some(&context),
    };
    let closure = ExportableRequirements::from_lock(
        &context_target,
        &[],
        &production_extras,
        &production_groups,
        false,
        &install_options,
    )?;
    for name in &selected {
        if !closure.contains_package(name) {
            bail!("Package `{name}` is not reachable from the selected resolution roots");
        }
    }
    Ok(Some(context))
}

/// Render one selection from a shared lockfile, deferring its file write until validation completes.
#[expect(clippy::fn_params_excessive_bools)]
async fn render_export<'output>(
    target: &ExportTarget,
    lock: &Lock,
    format: Option<ExportFormat>,
    all_packages: bool,
    package: &[PackageName],
    resolution_root: &[PackageName],
    prune: &[PackageName],
    hashes: bool,
    install_options: &InstallOptions,
    output_file: Option<&'output Path>,
    extras: &ExtrasSpecificationWithDefaults,
    groups: &DependencyGroupsWithDefaults,
    editable: Option<EditableMode>,
    include_annotations: bool,
    include_header: bool,
    include_index_url: bool,
    include_find_links: bool,
    settings: &ResolverSettings,
    client_builder: &BaseClientBuilder<'_>,
    concurrency: &Concurrency,
    quiet: bool,
    cache: &Cache,
    preview: Preview,
) -> Result<OutputWriter<'output>> {
    let workspace = match target {
        ExportTarget::Project(project) => Some(project.workspace()),
        ExportTarget::Script(_) => None,
    };
    // Identify the installation target.
    let target = match target {
        ExportTarget::Project(VirtualProject::Project(project)) => {
            if all_packages {
                InstallTarget::Workspace {
                    workspace: project.workspace(),
                    lock,
                }
            } else {
                match package {
                    // By default, install the root project.
                    [] => InstallTarget::Project {
                        workspace: project.workspace(),
                        name: project.project_name(),
                        lock,
                    },
                    [name] => InstallTarget::Project {
                        workspace: project.workspace(),
                        name,
                        lock,
                    },
                    names => InstallTarget::Projects {
                        workspace: project.workspace(),
                        names,
                        lock,
                    },
                }
            }
        }
        ExportTarget::Project(VirtualProject::NonProject(workspace)) => {
            if all_packages {
                InstallTarget::NonProjectWorkspace { workspace, lock }
            } else {
                match package {
                    // By default, install the entire workspace.
                    [] => InstallTarget::NonProjectWorkspace { workspace, lock },
                    [name] => InstallTarget::Project {
                        workspace,
                        name,
                        lock,
                    },
                    names => InstallTarget::Projects {
                        workspace,
                        names,
                        lock,
                    },
                }
            }
        }
        ExportTarget::Script(script) => InstallTarget::Script { script, lock },
    };

    // Validate that the set of requested extras and development groups are defined in the lockfile.
    target.validate_extras(extras)?;
    target.validate_groups(groups)?;

    let context = if let Some(workspace) = workspace {
        select_export_context(workspace, &target, resolution_root, extras, groups)?
    } else {
        if !resolution_root.is_empty() {
            bail!("`--resolution-root` requires an explicit-root workspace");
        }
        None
    };
    let target = ContextualExportTarget {
        target,
        context: context.as_deref(),
    };

    if output_file
        .and_then(Path::file_name)
        .is_some_and(|name| name.eq_ignore_ascii_case("pyproject.toml"))
    {
        return Err(anyhow!(
            "`pyproject.toml` is not a supported output format for `{}` (supported formats: {})",
            "uv export".green(),
            ExportFormat::value_variants()
                .iter()
                .filter_map(clap::ValueEnum::to_possible_value)
                .map(|value| value.get_name().to_string())
                .join(", ")
        ));
    }

    // Write the resolved dependencies to the output channel.
    let mut writer = OutputWriter::new(!quiet || output_file.is_none(), output_file);

    // Determine the output format.
    let format = format.unwrap_or_else(|| {
        if output_file
            .and_then(Path::extension)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
        {
            ExportFormat::RequirementsTxt
        } else if output_file
            .and_then(Path::file_name)
            .and_then(OsStr::to_str)
            .is_some_and(is_pylock_toml)
        {
            ExportFormat::PylockToml
        } else {
            ExportFormat::RequirementsTxt
        }
    });

    // Skip conflict detection for CycloneDX exports, as SBOMs are meant to document all dependencies including conflicts.
    if !matches!(format, ExportFormat::CycloneDX1_5) {
        detect_conflicts(&target.target, extras, groups)?;
    }

    // If the user is exporting to PEP 751, ensure the filename matches the specification.
    if matches!(format, ExportFormat::PylockToml) {
        if let Some(file_name) = output_file
            .and_then(Path::file_name)
            .and_then(OsStr::to_str)
        {
            if !is_pylock_toml(file_name) {
                return Err(anyhow!(
                    "Expected the output filename to be `pylock.toml` or `pylock.<name>.toml`, where `<name>` is non-empty and contains no dots; found `{file_name}`",
                ));
            }
        }
    }

    // Generate the export.
    match format {
        ExportFormat::RequirementsTxt => {
            let export = RequirementsTxtExport::from_lock(
                &target,
                prune,
                extras,
                groups,
                include_annotations,
                editable,
                hashes,
                install_options,
            )?;

            if include_header {
                writeln!(
                    writer,
                    "{}",
                    "# This file was autogenerated by uv via the following command:".green()
                )?;
                writeln!(writer, "{}", format!("#    {}", cmd()).green())?;
            }

            let mut wrote_preamble = false;

            // If necessary, include the `--index-url` and `--extra-index-url` locations.
            if include_index_url {
                let mut seen = FxHashSet::default();
                let mut emitted_explicit_index = false;

                if let Some(index) = settings.index_locations.default_index() {
                    writeln!(writer, "--index-url {}", index.url().verbatim())?;
                    seen.insert(index.url());
                    wrote_preamble = true;
                    emitted_explicit_index |= index.explicit;
                }
                for index in settings
                    .index_locations
                    .implicit_indexes()
                    .chain(settings.index_locations.explicit_indexes())
                {
                    if seen.insert(index.url()) {
                        writeln!(writer, "--extra-index-url {}", index.url().verbatim())?;
                        wrote_preamble = true;
                    }
                    emitted_explicit_index |= index.explicit;
                }

                if emitted_explicit_index {
                    warn_user!(
                        "`requirements.txt` does not support per-package index pinning; explicit indexes were emitted globally via `--extra-index-url`."
                    );
                }
            }

            // If necessary, include the `--find-links` locations.
            if include_find_links {
                for flat_index in settings.index_locations.flat_indexes() {
                    writeln!(writer, "--find-links {}", flat_index.url().verbatim())?;
                    wrote_preamble = true;
                }
            }

            if wrote_preamble {
                writeln!(writer)?;
            }

            write!(writer, "{export}")?;
        }
        ExportFormat::PylockToml => {
            let mut export = PylockToml::from_lock(
                &target,
                prune,
                extras,
                groups,
                include_annotations,
                editable.as_ref(),
                install_options,
            )?;

            // Registries don't always provide hashes, but `packages.*.hashes` is a required
            // key in PEP 751, so we have to download and hash files with missing hashes.
            if export.has_missing_hashes() {
                let client = RegistryClientBuilder::new(client_builder.clone(), cache.clone())
                    .index_locations(settings.index_locations.clone())
                    .build()?;
                export
                    .generate_missing_hashes(&client, concurrency.downloads, target.install_path())
                    .await?;
            }

            if include_header {
                writeln!(
                    writer,
                    "{}",
                    "# This file was autogenerated by uv via the following command:".green()
                )?;
                writeln!(writer, "{}", format!("#    {}", cmd()).green())?;
            }
            write!(writer, "{}", export.to_toml()?)?;
        }
        ExportFormat::CycloneDX1_5 => {
            let export = cyclonedx_json::from_lock(
                &target,
                prune,
                extras,
                groups,
                include_annotations,
                hashes,
                install_options,
                preview,
                all_packages,
            )?;

            export.output_as_json_v1_5(&mut writer)?;
        }
    }

    Ok(writer)
}

/// Format the uv command used to generate the output file.
fn cmd() -> String {
    let args = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().to_string())
        .scan(None, move |skip_next, arg| {
            if matches!(skip_next, Some(true)) {
                // Reset state; skip this iteration.
                *skip_next = None;
                return Some(None);
            }

            // Always skip the `--upgrade` flag.
            if arg == "--upgrade" || arg == "-U" {
                *skip_next = None;
                return Some(None);
            }

            // Always skip the `--upgrade-package` and mark the next item to be skipped
            if arg == "--upgrade-package" || arg == "-P" {
                *skip_next = Some(true);
                return Some(None);
            }

            // Skip only this argument if option and value are together
            if arg.starts_with("--upgrade-package=") || arg.starts_with("-P") {
                // Reset state; skip this iteration.
                *skip_next = None;
                return Some(None);
            }

            // Always skip the `--upgrade-group` and mark the next item to be skipped
            if arg == "--upgrade-group" {
                *skip_next = Some(true);
                return Some(None);
            }

            // Skip only this argument if option and value are together
            if arg.starts_with("--upgrade-group=") {
                // Reset state; skip this iteration.
                *skip_next = None;
                return Some(None);
            }

            // Always skip the `--quiet` flag.
            if arg == "--quiet" || arg == "-q" {
                *skip_next = None;
                return Some(None);
            }

            // Always skip the `--verbose` flag.
            if arg == "--verbose" || arg == "-v" {
                *skip_next = None;
                return Some(None);
            }

            // Return the argument.
            Some(Some(arg))
        })
        .flatten()
        .join(" ");
    format!("uv {args}")
}
