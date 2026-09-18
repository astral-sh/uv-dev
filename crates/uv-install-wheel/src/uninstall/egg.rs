use std::io;
use std::path::{Path, PathBuf};

use uv_fs::{Simplified, normalize_path, verbatim_path};

use crate::Layout;
use crate::wheel::reserved_script_name;

/// The subset of the selected installation scheme that can authorize a removal.
#[derive(Clone, Copy)]
pub(super) enum PathScope {
    Installation,
    Library,
    Scripts,
}

/// The result of checking the actual path that would be passed to an unlink operation.
#[derive(Debug)]
pub(super) enum PathDecision {
    Allowed(PathBuf),
    Missing,
    Escapes,
    Protected,
}

#[derive(Clone, Copy)]
enum RootKind {
    Purelib,
    Platlib,
    Scripts,
    Data,
    Include,
}

impl RootKind {
    fn permits(self, scope: PathScope) -> bool {
        match scope {
            PathScope::Installation => true,
            PathScope::Library => matches!(self, Self::Purelib | Self::Platlib),
            PathScope::Scripts => matches!(self, Self::Scripts),
        }
    }
}

#[derive(Clone)]
struct RootPaths {
    lexical: PathBuf,
    resolved: Option<PathBuf>,
}

impl RootPaths {
    fn new(path: &Path) -> io::Result<Self> {
        let absolute = absolute(path)?;
        Ok(Self {
            lexical: lexical(&absolute),
            resolved: canonicalize_optional(&absolute)?,
        })
    }

    fn contains_lexical(&self, path: &Path) -> bool {
        path != self.lexical && path.starts_with(&self.lexical)
    }

    fn contains_resolved_parent(&self, path: &Path) -> bool {
        self.resolved
            .as_ref()
            .is_some_and(|root| path.starts_with(root))
    }
}

struct InstallationRoot {
    kind: RootKind,
    paths: RootPaths,
}

struct ProtectedFile {
    absolute: PathBuf,
    lexical: PathBuf,
    resolved: Option<PathBuf>,
    exists: bool,
}

impl ProtectedFile {
    fn new(path: &Path) -> io::Result<Self> {
        let absolute = absolute(path)?;
        let name = absolute.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "protected file has no name")
        })?;
        let resolved = resolve_parent(&absolute)?.map(|parent| parent.join(name));
        let exists = match fs_err::metadata(&absolute) {
            Ok(_) => true,
            Err(err) if err.kind() == io::ErrorKind::NotFound => false,
            Err(err) => return Err(err),
        };
        Ok(Self {
            lexical: lexical(&absolute),
            absolute,
            resolved,
            exists,
        })
    }
}

/// Authority for legacy egg removals, derived only from the selected interpreter and scheme.
///
/// Resolving parents permits unlinking a final symlink without granting its target authority over
/// the environment. These checks do not make a subsequent path-based unlink race-free against an
/// unrelated process replacing an ancestor.
pub(super) struct EggUninstallAuthority {
    roots: Vec<InstallationRoot>,
    protected_files: Vec<ProtectedFile>,
    protected_scripts: Vec<RootPaths>,
    interpreter_names: Vec<String>,
    windows: bool,
}

impl EggUninstallAuthority {
    pub(super) fn new(layout: &Layout) -> io::Result<Self> {
        let roots = [
            (RootKind::Purelib, layout.scheme.purelib.as_path()),
            (RootKind::Platlib, layout.scheme.platlib.as_path()),
            (RootKind::Scripts, layout.scheme.scripts.as_path()),
            (RootKind::Data, layout.scheme.data.as_path()),
            (RootKind::Include, layout.scheme.include.as_path()),
        ]
        .into_iter()
        .map(|(kind, path)| {
            Ok(InstallationRoot {
                kind,
                paths: RootPaths::new(path)?,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;

        // Redirected schemes permit removals in a different installation. They do not replace the
        // selected interpreter's prefix, executable identities, or original scripts directory.
        let windows = layout.os_name == "nt";
        let interpreter_names = interpreter_names(layout.python_version, windows);
        let protected_scripts = [
            layout.interpreter_scripts.as_path(),
            layout.scheme.scripts.as_path(),
        ]
        .into_iter()
        .map(RootPaths::new)
        .collect::<io::Result<Vec<_>>>()?;
        let mut protected = vec![
            layout.sys_executable.clone(),
            layout.real_executable.clone(),
            layout.sys_prefix.join("pyvenv.cfg"),
            layout.scheme.data.join("pyvenv.cfg"),
        ];
        protected.extend(layout.sys_base_executable.iter().cloned());
        for scripts in [&layout.interpreter_scripts, &layout.scheme.scripts] {
            protected.extend(interpreter_names.iter().map(|name| scripts.join(name)));
        }
        protected.sort();
        protected.dedup();
        let protected_files = protected
            .iter()
            .map(|path| ProtectedFile::new(path))
            .collect::<io::Result<Vec<_>>>()?;

        Ok(Self {
            roots,
            protected_files,
            protected_scripts,
            interpreter_names,
            windows,
        })
    }

    pub(super) fn check(&self, path: &Path, scope: PathScope) -> io::Result<PathDecision> {
        let absolute = absolute(path)?;
        let lexical = lexical(&absolute);
        if !self
            .roots
            .iter()
            .any(|root| root.kind.permits(scope) && root.paths.contains_lexical(&lexical))
            || self.is_root(&lexical, false)
        {
            return Ok(PathDecision::Escapes);
        }

        // A missing interpreter alias must remain reserved even when its parent is not present.
        if self
            .protected_files
            .iter()
            .any(|core| protected_paths_equal(&lexical, &core.lexical))
            || self.is_interpreter_name(&lexical, false)
        {
            return Ok(PathDecision::Protected);
        }

        let Some(parent) = resolve_parent(&absolute)? else {
            return Ok(PathDecision::Missing);
        };
        let Some(name) = absolute.file_name() else {
            return Ok(PathDecision::Escapes);
        };
        let resolved = parent.join(name);
        if !self.roots.iter().any(|root| {
            root.kind.permits(scope)
                && root.paths.contains_lexical(&lexical)
                && root.paths.contains_resolved_parent(&parent)
        }) || self.is_root(&resolved, true)
        {
            return Ok(PathDecision::Escapes);
        }

        if self.protected_files.iter().any(|core| {
            core.resolved
                .as_ref()
                .is_some_and(|path| protected_paths_equal(&resolved, path))
        }) || self.is_interpreter_name(&resolved, true)
        {
            return Ok(PathDecision::Protected);
        }

        // Path spelling alone cannot identify a hardlink or an unusual interpreter alias. Only
        // existing protected files need an identity comparison; absent paths are covered above.
        for core in self.protected_files.iter().filter(|core| core.exists) {
            match same_file(&absolute, &core.absolute) {
                Ok(true) => return Ok(PathDecision::Protected),
                Ok(false) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }

        Ok(PathDecision::Allowed(absolute))
    }

    /// Check every existing entry that a recursive directory removal could delete. A directory
    /// symlink is an unlink candidate, not a traversal root. `None` means the original path was
    /// absent; a path that disappears after its initial observation does not authorize new paths.
    pub(super) fn check_directory_tree(
        &self,
        path: &Path,
        scope: PathScope,
    ) -> io::Result<Option<PathDecision>> {
        let absolute = absolute(path)?;
        let metadata = match fs_err::symlink_metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        let path = match self.check(&absolute, scope)? {
            PathDecision::Allowed(path) => path,
            decision => return Ok(Some(decision)),
        };

        let mut directories = Vec::new();
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            directories.push(path.clone());
        }
        while let Some(directory) = directories.pop() {
            let metadata = match fs_err::symlink_metadata(&directory) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let directory = match self.check(&directory, scope)? {
                PathDecision::Allowed(directory) => directory,
                PathDecision::Missing => continue,
                decision => return Ok(Some(decision)),
            };
            let entries = match fs_err::read_dir(&directory) {
                Ok(entries) => entries,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            for entry in entries {
                let entry = entry?.path();
                let metadata = match fs_err::symlink_metadata(&entry) {
                    Ok(metadata) => metadata,
                    Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                    Err(err) => return Err(err),
                };
                let entry = match self.check(&entry, scope)? {
                    PathDecision::Allowed(entry) => entry,
                    PathDecision::Missing => continue,
                    decision => return Ok(Some(decision)),
                };
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    directories.push(entry);
                }
            }
        }

        Ok(Some(PathDecision::Allowed(path)))
    }

    /// Check an empty directory before pruning it. A directory link is not a pruning root.
    pub(super) fn prunable_directory(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        let PathDecision::Allowed(path) = self.check(path, PathScope::Installation)? else {
            return Ok(None);
        };
        let metadata = match fs_err::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Ok(None);
        }
        let Some(resolved) = canonicalize_optional(&path)? else {
            return Ok(None);
        };
        let lexical = lexical(&path);
        if self.is_root(&resolved, true)
            || !self.roots.iter().any(|root| {
                root.paths.contains_lexical(&lexical)
                    && root.paths.contains_resolved_parent(&resolved)
            })
        {
            return Ok(None);
        }
        Ok(Some(path))
    }

    fn is_root(&self, path: &Path, resolved: bool) -> bool {
        self.roots.iter().any(|root| {
            if resolved {
                root.paths
                    .resolved
                    .as_ref()
                    .is_some_and(|root| protected_paths_equal(path, root))
            } else {
                protected_paths_equal(path, &root.paths.lexical)
            }
        })
    }

    fn is_interpreter_name(&self, path: &Path, resolved: bool) -> bool {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|s| s.to_str()))
        else {
            return false;
        };
        if !self.protected_scripts.iter().any(|root| {
            if resolved {
                root.resolved
                    .as_ref()
                    .is_some_and(|root| protected_paths_equal(parent, root))
            } else {
                protected_paths_equal(parent, &root.lexical)
            }
        }) {
            return false;
        }
        let name = name.to_ascii_lowercase();
        let reserved_name = if self.windows {
            name.strip_suffix(".exe").unwrap_or(&name)
        } else {
            &name
        };
        reserved_script_name(reserved_name).is_some()
            || self.interpreter_names.iter().any(|alias| alias == &name)
    }
}

/// Return the selected version's launcher names emitted by `uv-virtualenv`. Windows launchers can
/// be distinct copies, so file identity with `sys.executable` alone is insufficient.
fn interpreter_names((major, minor): (u8, u8), windows: bool) -> Vec<String> {
    let mut names = vec![
        "python".to_string(),
        format!("python{major}"),
        format!("python{major}.{minor}"),
        format!("python{major}.{minor}t"),
        "pypy".to_string(),
        format!("pypy{major}"),
        "graalpy".to_string(),
    ];
    if windows {
        names.extend([
            "pythonw".to_string(),
            format!("pythonw{major}.{minor}t"),
            format!("pypy{major}.{minor}"),
            "pypyw".to_string(),
            format!("pypy{major}.{minor}w"),
        ]);
        for name in &mut names {
            name.push_str(".exe");
        }
    }
    names
}

fn absolute(path: &Path) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty uninstall path",
        ));
    }
    std::path::absolute(path)
}

fn lexical(path: &Path) -> PathBuf {
    // Use one prefix form for comparisons without changing the path passed to filesystem APIs.
    // A verbatim Windows filename can contain meaningful trailing spaces or dots.
    let normalized = normalize_path(path.simplified());
    verbatim_path(normalized.as_ref()).into_owned()
}

fn canonicalize_optional(path: &Path) -> io::Result<Option<PathBuf>> {
    match path.simple_canonicalize() {
        Ok(path) => Ok(Some(lexical(&path))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn resolve_parent(path: &Path) -> io::Result<Option<PathBuf>> {
    match path.parent() {
        Some(parent) => canonicalize_optional(parent),
        None => Ok(None),
    }
}

/// Case-insensitive spelling is an additional protection, never permission to leave a root.
fn protected_paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.as_os_str()
            .as_encoded_bytes()
            .eq_ignore_ascii_case(right.as_os_str().as_encoded_bytes())
    } else {
        left == right
    }
}

#[cfg(unix)]
fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    // Unlinking an owned file does not require permission to read its contents.
    let left = fs_err::metadata(left)?;
    let right = fs_err::metadata(right)?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(not(unix))]
fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    same_file::is_same_file(left, right)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use uv_pypi_types::Scheme;

    use super::{EggUninstallAuthority, PathDecision, PathScope, interpreter_names};
    use crate::uninstall::uninstall_egg;
    use crate::{Error, Layout};

    fn layout(root: &Path) -> Layout {
        let scripts = root.join(if cfg!(windows) { "Scripts" } else { "bin" });
        let python = scripts.join(if cfg!(windows) {
            "python.exe"
        } else {
            "python"
        });
        let site_packages = root.join("lib/python3.13/site-packages");
        Layout {
            sys_executable: python.clone(),
            sys_prefix: root.to_path_buf(),
            sys_base_executable: None,
            real_executable: python,
            interpreter_scripts: scripts.clone(),
            python_version: (3, 13),
            os_name: if cfg!(windows) { "nt" } else { "posix" }.to_string(),
            scheme: Scheme {
                purelib: site_packages.clone(),
                platlib: site_packages,
                scripts,
                data: root.to_path_buf(),
                include: root.join("include/python3.13"),
            },
        }
    }

    fn target_layout(selected: &Layout, root: &Path) -> Layout {
        let mut layout = selected.clone();
        layout.scheme = Scheme {
            purelib: root.to_path_buf(),
            platlib: root.to_path_buf(),
            scripts: root.join("bin"),
            data: root.to_path_buf(),
            include: root.join("include"),
        };
        layout
    }

    fn write(path: impl AsRef<Path>, contents: &str) {
        let path = path.as_ref();
        fs_err::create_dir_all(path.parent().unwrap()).unwrap();
        fs_err::write(path, contents).unwrap();
    }

    fn initialize(layout: &Layout) {
        for root in [
            &layout.scheme.purelib,
            &layout.scheme.platlib,
            &layout.scheme.scripts,
            &layout.scheme.data,
            &layout.scheme.include,
        ] {
            fs_err::create_dir_all(root).unwrap();
        }
        write(&layout.sys_executable, "selected interpreter");
        write(
            layout.sys_prefix.join("pyvenv.cfg"),
            "selected configuration",
        );
    }

    fn recorded_egg(layout: &Layout, record: &str) -> PathBuf {
        let egg_info = layout.scheme.purelib.join("owned-0.1.0.egg-info");
        write(
            egg_info.join("PKG-INFO"),
            "Metadata-Version: 2.1\nName: owned\nVersion: 0.1.0\n",
        );
        write(egg_info.join("top_level.txt"), "owned\n");
        write(egg_info.join("installed-files.txt"), record);
        egg_info
    }

    #[test]
    fn selected_interpreter_is_not_an_installation_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let mut redirected = selected.clone();
        redirected.scheme = layout(&temp.join("redirected")).scheme;
        fs_err::create_dir_all(&redirected.scheme.purelib).unwrap();
        let authority = EggUninstallAuthority::new(&redirected).unwrap();

        assert!(matches!(
            authority
                .check(&selected.sys_executable, PathScope::Installation)
                .unwrap(),
            PathDecision::Escapes
        ));
        assert!(matches!(
            authority
                .check(
                    &redirected.scheme.purelib.join("owned.py"),
                    PathScope::Installation,
                )
                .unwrap(),
            PathDecision::Allowed(_)
        ));
        assert!(matches!(
            authority
                .check(&redirected.scheme.purelib, PathScope::Installation)
                .unwrap(),
            PathDecision::Escapes
        ));
    }

    #[test]
    fn metadata_outside_the_selected_library_cannot_authorize_removals() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let payload = selected.scheme.purelib.join("owned.py");
        write(&payload, "owned payload");
        let egg_info = temp.join("outside/owned-0.1.0.egg-info");
        write(
            egg_info.join("installed-files.txt"),
            &format!("{}\n", payload.display()),
        );

        assert!(matches!(
            uninstall_egg(&egg_info, "owned 0.1.0", &selected),
            Err(Error::BrokenVenv(_))
        ));

        assert!(payload.exists());
        assert!(egg_info.exists());
    }

    #[test]
    fn metadata_tree_cannot_contain_the_selected_interpreter() {
        let temp = assert_fs::TempDir::new().unwrap();
        let target = temp.join("target");
        let selected = layout(&target.join("owned-0.1.0.egg-info"));
        initialize(&selected);
        let targeted = target_layout(&selected, &target);
        initialize(&targeted);
        let payload = target.join("payload.py");
        write(&payload, "recorded payload");
        let launcher =
            targeted
                .scheme
                .scripts
                .join(if cfg!(windows) { "tool.exe" } else { "tool" });
        write(&launcher, "owned launcher");
        let egg_info = recorded_egg(&targeted, "../payload.py\n");
        write(
            egg_info.join("entry_points.txt"),
            "[console_scripts]\ntool = missing:main\n",
        );

        assert!(matches!(
            uninstall_egg(&egg_info, "owned 0.1.0", &targeted),
            Err(Error::BrokenVenv(message)) if message.contains("core Python environment file")
        ));

        assert_eq!(
            fs_err::read_to_string(&selected.sys_executable).unwrap(),
            "selected interpreter"
        );
        assert_eq!(
            fs_err::read_to_string(selected.sys_prefix.join("pyvenv.cfg")).unwrap(),
            "selected configuration"
        );
        assert_eq!(
            fs_err::read_to_string(&payload).unwrap(),
            "recorded payload"
        );
        assert_eq!(fs_err::read_to_string(&launcher).unwrap(), "owned launcher");
        assert!(egg_info.exists());
    }

    #[test]
    fn metadata_tree_cannot_contain_a_selected_installation_root() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let mut targeted = target_layout(&selected, &temp.join("target"));
        targeted.scheme.include = targeted.scheme.purelib.join("owned-0.1.0.egg-info/include");
        initialize(&targeted);
        let payload = targeted.scheme.purelib.join("payload.py");
        write(&payload, "recorded payload");
        write(targeted.scheme.include.join("sentinel"), "selected root");
        let egg_info = recorded_egg(&targeted, "../payload.py\n");

        assert!(matches!(
            uninstall_egg(&egg_info, "owned 0.1.0", &targeted),
            Err(Error::BrokenVenv(message)) if message.contains("installation root")
        ));

        assert_eq!(
            fs_err::read_to_string(&payload).unwrap(),
            "recorded payload"
        );
        assert_eq!(
            fs_err::read_to_string(targeted.scheme.include.join("sentinel")).unwrap(),
            "selected root"
        );
        assert!(selected.sys_executable.exists());
        assert!(egg_info.exists());
    }

    #[test]
    fn metadata_core_alias_stops_before_fallback_payload_removal() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let targeted = target_layout(&selected, &temp.join("target"));
        initialize(&targeted);
        let payload = targeted.scheme.purelib.join("owned.py");
        write(&payload, "fallback payload");
        let egg_info = targeted.scheme.purelib.join("owned-0.1.0.egg-info");
        write(egg_info.join("top_level.txt"), "owned\n");
        let alias = egg_info.join("interpreter-alias");
        fs_err::hard_link(&selected.sys_executable, &alias).unwrap();

        assert!(matches!(
            uninstall_egg(&egg_info, "owned 0.1.0", &targeted),
            Err(Error::BrokenVenv(message)) if message.contains("core Python environment file")
        ));

        assert_eq!(
            fs_err::read_to_string(&payload).unwrap(),
            "fallback payload"
        );
        assert!(same_file::is_same_file(&selected.sys_executable, &alias).unwrap());
        assert!(egg_info.exists());
    }

    #[cfg(unix)]
    #[test]
    fn metadata_tree_io_error_precedes_payload_removal() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let payload = selected.scheme.purelib.join("payload.py");
        write(&payload, "recorded payload");
        let egg_info = recorded_egg(&selected, "../payload.py\n");
        let invalid = egg_info.join("loop");
        fs_err::os::unix::fs::symlink("loop", &invalid).unwrap();

        let Err(Error::Io(error)) = uninstall_egg(&egg_info, "owned 0.1.0", &selected) else {
            panic!("expected the metadata symlink-loop I/O error");
        };

        assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(
            fs_err::read_to_string(&payload).unwrap(),
            "recorded payload"
        );
        assert!(
            fs_err::symlink_metadata(&invalid)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(egg_info.exists());
    }

    #[test]
    fn absent_metadata_keeps_the_earlier_missing_top_level_error() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let egg_info = selected.scheme.purelib.join("absent-0.1.0.egg-info");
        let authority = EggUninstallAuthority::new(&selected).unwrap();

        assert!(
            authority
                .check_directory_tree(&egg_info, PathScope::Library)
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            uninstall_egg(&egg_info, "absent 0.1.0", &selected),
            Err(Error::MissingTopLevel(path)) if path == egg_info.join("top_level.txt")
        ));
        assert!(selected.sys_executable.exists());
    }

    #[test]
    fn an_empty_record_does_not_fall_back_to_top_level() {
        for record in ["", "\r\n\r\n"] {
            let temp = assert_fs::TempDir::new().unwrap();
            let selected = layout(&temp.join("selected"));
            initialize(&selected);
            let payload = selected.scheme.purelib.join("owned/__init__.py");
            write(&payload, "unrecorded payload");
            let egg_info = recorded_egg(&selected, record);

            uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

            assert!(payload.exists());
            assert!(!egg_info.exists());
        }
    }

    #[test]
    fn a_recorded_directory_is_not_recursively_removed() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let payload = selected.scheme.purelib.join("owned/sibling.py");
        write(&payload, "unrecorded payload");
        let egg_info = recorded_egg(&selected, "../owned\n");

        assert!(matches!(
            uninstall_egg(&egg_info, "owned 0.1.0", &selected),
            Err(Error::Io(_))
        ));

        assert!(payload.exists());
        assert!(egg_info.exists());
    }

    #[test]
    fn recorded_filename_whitespace_is_literal() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let ordinary = selected.scheme.purelib.join("owned.py");
        let spaced = uv_fs::verbatim_path(&selected.scheme.purelib.join("owned.py ")).into_owned();
        write(&ordinary, "unrecorded sibling");
        write(&spaced, "recorded filename");
        let egg_info = recorded_egg(&selected, &format!("{}\r\n", spaced.display()));

        uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert!(fs_err::symlink_metadata(&spaced).is_err());
        assert_eq!(
            fs_err::read_to_string(&ordinary).unwrap(),
            "unrecorded sibling"
        );
        assert!(!egg_info.exists());
    }

    #[test]
    fn recorded_files_protect_selected_identities_with_redirected_roots() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let mut redirected = selected.clone();
        redirected.scheme = layout(&temp.join("redirected")).scheme;
        fs_err::create_dir_all(&redirected.scheme.purelib).unwrap();
        redirected.real_executable = temp.join("queried/odd-python");
        redirected.sys_base_executable = Some(temp.join("base/odd-python"));
        write(&redirected.real_executable, "queried interpreter");
        write(
            redirected.sys_base_executable.as_ref().unwrap(),
            "base interpreter",
        );
        write(
            redirected.scheme.data.join("pyvenv.cfg"),
            "redirected configuration",
        );
        let payload = redirected.scheme.purelib.join("owned.py");
        write(&payload, "owned payload");
        let aliases = [
            ("selected-link", &redirected.sys_executable),
            ("queried-link", &redirected.real_executable),
            (
                "base-link",
                redirected.sys_base_executable.as_ref().unwrap(),
            ),
        ];
        for (name, core) in aliases {
            fs_err::hard_link(core, redirected.scheme.purelib.join(name)).unwrap();
        }
        let egg_info = recorded_egg(
            &redirected,
            &format!(
                "../owned.py\n../selected-link\n../queried-link\n../base-link\n{}\n{}\n",
                redirected.scheme.data.join("pyvenv.cfg").display(),
                selected.sys_prefix.join("pyvenv.cfg").display(),
            ),
        );

        uninstall_egg(&egg_info, "owned 0.1.0", &redirected).unwrap();

        assert!(!payload.exists());
        assert!(!egg_info.exists());
        for (name, core) in aliases {
            assert!(redirected.scheme.purelib.join(name).exists());
            assert!(core.exists());
        }
        assert_eq!(
            fs_err::read_to_string(redirected.scheme.data.join("pyvenv.cfg")).unwrap(),
            "redirected configuration"
        );
        assert_eq!(
            fs_err::read_to_string(selected.sys_prefix.join("pyvenv.cfg")).unwrap(),
            "selected configuration"
        );
    }

    #[test]
    fn trusted_root_alias_does_not_authorize_a_child_alias() {
        let temp = assert_fs::TempDir::new().unwrap();
        let mut selected = layout(&temp.join("selected"));
        initialize(&selected);
        let trusted = temp.join("trusted-library");
        let outside = temp.join("outside");
        fs_err::create_dir_all(&trusted).unwrap();
        fs_err::create_dir_all(&outside).unwrap();
        let alias = selected.scheme.data.join("library-alias");
        uv_fs::create_symlink(&trusted, &alias).unwrap();
        selected.scheme.purelib.clone_from(&alias);
        selected.scheme.platlib = alias;
        uv_fs::create_symlink(&outside, trusted.join("child-alias")).unwrap();
        write(trusted.join("owned.py"), "owned payload");
        write(outside.join("sentinel.py"), "outside sentinel");
        let egg_info = recorded_egg(
            &selected,
            "../owned.py\n../child-alias/sentinel.py\n../child-alias/missing/absent.py\n",
        );

        uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert!(!trusted.join("owned.py").exists());
        assert!(!egg_info.exists());
        assert!(selected.scheme.purelib.exists());
        assert_eq!(
            fs_err::read_to_string(outside.join("sentinel.py")).unwrap(),
            "outside sentinel"
        );

        // A missing parent does not grant authority to a file that appears on a later invocation.
        write(outside.join("missing/absent.py"), "later outside sentinel");
        let authority = EggUninstallAuthority::new(&selected).unwrap();
        assert!(matches!(
            authority
                .check(
                    &selected
                        .scheme
                        .purelib
                        .join("child-alias/missing/absent.py"),
                    PathScope::Installation,
                )
                .unwrap(),
            PathDecision::Escapes
        ));
        assert!(outside.join("missing/absent.py").exists());
    }

    #[cfg(unix)]
    #[test]
    fn recorded_final_symlink_is_unlinked_without_following_its_target() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let target = temp.join("outside/sentinel.py");
        write(&target, "outside sentinel");
        let link = selected.scheme.purelib.join("owned.py");
        fs_err::os::unix::fs::symlink(&target, &link).unwrap();
        let core_link = selected.scheme.purelib.join("core.py");
        fs_err::os::unix::fs::symlink(&selected.sys_executable, &core_link).unwrap();
        let egg_info = recorded_egg(&selected, "../owned.py\n../core.py\n");

        uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert!(fs_err::symlink_metadata(&link).is_err());
        assert!(core_link.exists());
        assert_eq!(fs_err::read_to_string(&target).unwrap(), "outside sentinel");
        assert!(selected.sys_executable.exists());
    }

    #[test]
    fn launcher_child_alias_fails_before_removing_recorded_payload() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let payload = selected.scheme.purelib.join("owned.py");
        write(&payload, "owned payload");
        let outside = temp.join("outside");
        for name in ["tool", "tool.exe", "tool.exe.manifest", "tool-script.py"] {
            write(outside.join(name), "outside launcher");
        }
        uv_fs::create_symlink(&outside, selected.scheme.scripts.join("nested")).unwrap();
        let egg_info = recorded_egg(&selected, "../owned.py\n");
        write(
            egg_info.join("entry_points.txt"),
            "[console_scripts]\nnested/tool = missing:main\n",
        );

        assert!(matches!(
            uninstall_egg(&egg_info, "owned 0.1.0", &selected),
            Err(Error::InvalidWheel(message)) if message.contains("within the scripts directory")
        ));

        assert!(payload.exists());
        assert!(egg_info.exists());
        for name in ["tool", "tool.exe", "tool.exe.manifest", "tool-script.py"] {
            assert_eq!(
                fs_err::read_to_string(outside.join(name)).unwrap(),
                "outside launcher"
            );
        }
    }

    #[test]
    fn fallback_final_directory_link_does_not_remove_its_target() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let outside = temp.join("outside");
        write(outside.join("sentinel.py"), "outside sentinel");
        let link = selected.scheme.purelib.join("owned");
        uv_fs::create_symlink(&outside, &link).unwrap();
        let egg_info = selected.scheme.purelib.join("owned-0.1.0.egg-info");
        write(egg_info.join("top_level.txt"), "owned\n");

        uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert!(fs_err::symlink_metadata(&link).is_err());
        assert!(!egg_info.exists());
        assert_eq!(
            fs_err::read_to_string(outside.join("sentinel.py")).unwrap(),
            "outside sentinel"
        );
    }

    #[test]
    fn fallback_preserves_overlapping_target_roots() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let targeted = target_layout(&selected, &temp.join("target"));
        initialize(&targeted);
        let target = &targeted.scheme.purelib;
        for name in ["bin", "include"] {
            write(target.join(name).join("sentinel"), "installation root");
            write(
                target.join(format!("{name}.py")),
                "unselected adjacent module",
            );
        }
        write(target.join("owned/__init__.py"), "owned package");
        write(target.join("owned.py"), "unselected adjacent module");
        write(target.join("shared/sibling.py"), "namespace sibling");
        for extension in ["py", "pyc", "pyo"] {
            write(target.join(format!("module.{extension}")), "owned module");
        }
        let egg_info = target.join("owned-0.1.0.egg-info");
        write(
            egg_info.join("top_level.txt"),
            "bin\ninclude\nowned\nmodule\nshared\n",
        );
        write(egg_info.join("namespace_packages.txt"), "shared\n");

        uninstall_egg(&egg_info, "owned 0.1.0", &targeted).unwrap();

        for name in ["bin", "include"] {
            assert_eq!(
                fs_err::read_to_string(target.join(name).join("sentinel")).unwrap(),
                "installation root"
            );
            assert!(target.join(format!("{name}.py")).exists());
        }
        assert!(!target.join("owned").exists());
        assert!(target.join("owned.py").exists());
        assert!(target.join("shared/sibling.py").exists());
        for extension in ["py", "pyc", "pyo"] {
            assert!(!target.join(format!("module.{extension}")).exists());
        }
        assert!(!egg_info.exists());
        assert!(selected.sys_executable.exists());
        assert!(selected.sys_prefix.join("pyvenv.cfg").exists());
    }

    #[test]
    fn fallback_missing_target_root_selects_adjacent_modules() {
        for mask in 0..8 {
            let temp = assert_fs::TempDir::new().unwrap();
            let selected = layout(&temp.join("selected"));
            initialize(&selected);
            let targeted = target_layout(&selected, &temp.join("target"));
            initialize(&targeted);
            fs_err::remove_dir(&targeted.scheme.scripts).unwrap();
            let mut expected_files = 0;
            for (index, extension) in ["py", "pyc", "pyo"].into_iter().enumerate() {
                if mask & (1 << index) != 0 {
                    write(
                        targeted.scheme.purelib.join(format!("bin.{extension}")),
                        "owned module",
                    );
                    expected_files += 1;
                }
            }
            let sibling = targeted.scheme.purelib.join("sibling.py");
            write(&sibling, "unrelated module");
            let egg_info = targeted.scheme.purelib.join("owned-0.1.0.egg-info");
            write(egg_info.join("top_level.txt"), "bin\n");

            let removed = uninstall_egg(&egg_info, "owned 0.1.0", &targeted).unwrap();

            assert_eq!(removed.file_count, expected_files, "mask: {mask}");
            for extension in ["py", "pyc", "pyo"] {
                assert!(
                    !targeted
                        .scheme
                        .purelib
                        .join(format!("bin.{extension}"))
                        .exists()
                );
            }
            assert!(!targeted.scheme.scripts.exists());
            assert!(targeted.scheme.include.is_dir());
            assert!(sibling.exists());
            assert!(!egg_info.exists());
            assert!(selected.sys_executable.exists());
        }
    }

    #[test]
    fn fallback_duplicate_directories_do_not_select_adjacent_files() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let library = &selected.scheme.purelib;
        write(library.join("owned/__init__.py"), "owned package");
        for extension in ["py", "pyc", "pyo"] {
            write(
                library.join(format!("owned.{extension}")),
                "unselected adjacent module",
            );
        }
        let egg_info = library.join("owned-0.1.0.egg-info");
        write(egg_info.join("top_level.txt"), "owned\nowned\n");

        let removed = uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert_eq!(removed.file_count, 0);
        assert_eq!(removed.dir_count, 2);
        assert!(!library.join("owned").exists());
        for extension in ["py", "pyc", "pyo"] {
            assert!(library.join(format!("owned.{extension}")).exists());
        }
        assert!(!egg_info.exists());
    }

    #[test]
    fn fallback_recursive_removal_preserves_nested_roots_and_core_aliases() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let mut targeted = target_layout(&selected, &temp.join("target"));
        targeted.scheme.include = targeted.scheme.purelib.join("package/inner/include");
        initialize(&targeted);
        let target = &targeted.scheme.purelib;
        write(targeted.scheme.include.join("sentinel"), "nested root");
        write(target.join("package/payload.py"), "protected subtree");
        write(target.join("package.py"), "unselected adjacent module");
        write(target.join("corepackage/payload.py"), "protected subtree");
        write(target.join("corepackage.py"), "unselected adjacent module");
        let alias = target.join("corepackage/interpreter-alias");
        fs_err::hard_link(&selected.sys_executable, &alias).unwrap();
        write(target.join("ordinary.py"), "owned module");
        let egg_info = target.join("owned-0.1.0.egg-info");
        write(
            egg_info.join("top_level.txt"),
            "package\ncorepackage\nordinary\n",
        );

        uninstall_egg(&egg_info, "owned 0.1.0", &targeted).unwrap();

        assert_eq!(
            fs_err::read_to_string(targeted.scheme.include.join("sentinel")).unwrap(),
            "nested root"
        );
        for path in [
            "package/payload.py",
            "package.py",
            "corepackage/payload.py",
            "corepackage.py",
        ] {
            assert!(target.join(path).exists());
        }
        assert!(same_file::is_same_file(&selected.sys_executable, &alias).unwrap());
        assert!(!target.join("ordinary.py").exists());
        assert!(!egg_info.exists());
    }

    #[test]
    fn fallback_recursive_removal_does_not_follow_child_directory_links() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let outside = temp.join("outside");
        write(outside.join("sentinel.py"), "outside sentinel");
        write(
            selected.scheme.purelib.join("owned/module.py"),
            "owned module",
        );
        uv_fs::create_symlink(&outside, selected.scheme.purelib.join("owned/alias")).unwrap();
        let egg_info = selected.scheme.purelib.join("owned-0.1.0.egg-info");
        write(egg_info.join("top_level.txt"), "owned\n");

        uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert!(!selected.scheme.purelib.join("owned").exists());
        assert!(!egg_info.exists());
        assert_eq!(
            fs_err::read_to_string(outside.join("sentinel.py")).unwrap(),
            "outside sentinel"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fallback_preflight_error_precedes_all_payload_removal() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let earlier = selected.scheme.purelib.join("earlier.py");
        let later = selected.scheme.purelib.join("later.py");
        let invalid = selected.scheme.purelib.join("later.pyc");
        write(&earlier, "earlier payload");
        write(&later, "later payload");
        fs_err::os::unix::fs::symlink("later.pyc", &invalid).unwrap();
        let egg_info = selected.scheme.purelib.join("owned-0.1.0.egg-info");
        write(egg_info.join("top_level.txt"), "earlier\nlater\n");

        let Err(Error::Io(error)) = uninstall_egg(&egg_info, "owned 0.1.0", &selected) else {
            panic!("expected the symlink-loop I/O error");
        };

        assert_ne!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(earlier.exists());
        assert!(later.exists());
        assert!(
            fs_err::symlink_metadata(&invalid)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(egg_info.exists());
    }

    #[test]
    fn bytecode_alias_and_installation_roots_survive_pruning() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let outside = temp.join("outside");
        write(outside.join("module.cpython-313.pyc"), "outside bytecode");
        write(
            selected.scheme.purelib.join("owned/module.py"),
            "owned payload",
        );
        uv_fs::create_symlink(&outside, selected.scheme.purelib.join("owned/__pycache__")).unwrap();
        write(
            selected.scheme.include.join("empty-after.txt"),
            "owned include",
        );
        let egg_info = recorded_egg(
            &selected,
            &format!(
                "../owned/module.py\n{}\n",
                selected.scheme.include.join("empty-after.txt").display()
            ),
        );

        uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert!(!selected.scheme.purelib.join("owned/module.py").exists());
        assert!(selected.scheme.purelib.join("owned/__pycache__").exists());
        assert_eq!(
            fs_err::read_to_string(outside.join("module.cpython-313.pyc")).unwrap(),
            "outside bytecode"
        );
        for root in [
            &selected.scheme.purelib,
            &selected.scheme.platlib,
            &selected.scheme.scripts,
            &selected.scheme.data,
            &selected.scheme.include,
        ] {
            assert!(root.is_dir());
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_identity_does_not_require_content_read_permission() {
        use std::os::unix::fs::PermissionsExt;

        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let payload = selected.scheme.purelib.join("owned.py");
        write(&payload, "owned payload");
        fs_err::set_permissions(&payload, std::fs::Permissions::from_mode(0o0)).unwrap();
        let egg_info = recorded_egg(&selected, "../owned.py\n");

        uninstall_egg(&egg_info, "owned 0.1.0", &selected).unwrap();

        assert!(!payload.exists());
        assert!(selected.sys_executable.exists());
    }

    #[cfg(unix)]
    #[test]
    fn non_not_found_parent_errors_are_not_missing_paths() {
        let temp = assert_fs::TempDir::new().unwrap();
        let selected = layout(&temp.join("selected"));
        initialize(&selected);
        let file = selected.scheme.purelib.join("not-a-directory");
        write(&file, "ordinary file");
        let authority = EggUninstallAuthority::new(&selected).unwrap();
        assert_eq!(
            authority
                .check(&file.join("child"), PathScope::Installation)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotADirectory
        );
        assert!(matches!(
            authority
                .check(
                    &selected.scheme.include.join("absent/child"),
                    PathScope::Installation,
                )
                .unwrap(),
            PathDecision::Missing
        ));
    }

    #[test]
    fn windows_alias_inventory_matches_the_virtualenv_emitter() {
        let mut names = interpreter_names((3, 13), true);
        names.sort();
        assert_eq!(
            names,
            [
                "graalpy.exe",
                "pypy.exe",
                "pypy3.13.exe",
                "pypy3.13w.exe",
                "pypy3.exe",
                "pypyw.exe",
                "python.exe",
                "python3.13.exe",
                "python3.13t.exe",
                "python3.exe",
                "pythonw.exe",
                "pythonw3.13t.exe",
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_raw_interpreter_names_fail_before_payload_removal() {
        for name in interpreter_names((3, 13), true)
            .into_iter()
            .chain(["PyThOn.ExE".to_string()])
        {
            let temp = assert_fs::TempDir::new().unwrap();
            let selected = layout(&temp.join("selected"));
            initialize(&selected);
            let alias = selected.scheme.scripts.join(&name);
            if !alias.exists() {
                write(&alias, "distinct interpreter copy");
            }
            let original = fs_err::read(&alias).unwrap();
            let payload = selected.scheme.purelib.join("owned.py");
            write(&payload, "owned payload");
            let egg_info = recorded_egg(&selected, "../owned.py\n");
            write(
                egg_info.join("entry_points.txt"),
                &format!("[console_scripts]\n{name} = missing:main\n"),
            );

            assert!(matches!(
                uninstall_egg(&egg_info, "owned 0.1.0", &selected),
                Err(Error::InvalidWheel(message)) if message.contains("core Python environment file")
            ));

            assert!(payload.exists(), "{name}");
            assert!(egg_info.exists(), "{name}");
            assert_eq!(fs_err::read(&alias).unwrap(), original, "{name}");
        }
    }
}
