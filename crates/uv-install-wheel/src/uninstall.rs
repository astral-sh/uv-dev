use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::fmt::Display;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing::trace;

use uv_fs::write_atomic_sync;
use uv_pypi_types::Identifier;
use uv_warnings::warn_user;

use crate::wheel::read_record;
use crate::{Error, Layout};

/// Uninstall the wheel represented by the given `.dist-info` directory.
pub fn uninstall_wheel(
    dist_info: &Path,
    distribution: impl Display,
    layout: &Layout,
) -> Result<Uninstall, Error> {
    let Some(site_packages) = dist_info.parent() else {
        return Err(Error::BrokenVenv(
            "dist-info directory is not in a site-packages directory".to_string(),
        ));
    };

    // Read the RECORD file.
    let record = {
        let record_path = dist_info.join("RECORD");
        let record_file = match fs_err::File::open(&record_path) {
            Ok(record_file) => record_file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::MissingRecord(record_path));
            }
            Err(err) => return Err(err.into()),
        };
        read_record(record_file)?
    };

    let mut uninstall = Uninstall::default();
    let canonical_site_packages = canonicalize_directory(site_packages)?;
    let mut record_paths = RecordPathResolver::new(site_packages, layout);

    #[cfg(windows)]
    let itself = std::env::current_exe().ok();

    // Uninstall the files, keeping track of any directories that are left empty.
    let mut visited = BTreeSet::new();
    for entry in record {
        let Some(path) = record_paths.resolve(&entry.path, &distribution)? else {
            continue;
        };

        // On Windows, deleting the current executable is a special case.
        #[cfg(windows)]
        if let Some(itself) = itself.as_ref()
            && itself
                .file_name()
                .is_some_and(|name| path.file_name() == Some(name))
            && same_file::is_same_file(itself, &path).unwrap_or(false)
        {
            tracing::debug!("Detected self-delete of executable: {}", path.display());
            match self_replace::self_delete_outside_path(site_packages) {
                Ok(()) => uninstall.record_removed_wheel_file(path, &mut visited),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
            continue;
        }

        match fs_err::remove_file(&path) {
            Ok(()) => uninstall.record_removed_wheel_file(path, &mut visited),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                match contains_installation_scheme(&path, layout) {
                    Ok(true) => {
                        if should_warn_for_distribution(
                            &WARNED_FOR_RECORD_ENTRY_PACKAGE,
                            &distribution,
                        ) {
                            warn_user!(
                                "Invalid `RECORD` entry in `{distribution}` that would remove an installation directory, skipping: {}",
                                entry.path
                            );
                        }
                        continue;
                    }
                    Ok(false) => {}
                    Err(check_err) if check_err.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(err.into()),
                }
                match fs_err::remove_dir_all(&path) {
                    Ok(()) => uninstall.record_removed_directory(&path),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(err.into()),
                }
            }
        }
    }

    // If any directories were left empty, remove them. Iterate in reverse order such that we visit
    // the deepest directories first.
    for path in visited.into_iter().rev() {
        // No need to look at directories outside of `site-packages` (like `bin`).
        if !path.starts_with(&canonical_site_packages) {
            continue;
        }

        // Iterate up the directory tree, removing any empty directories. It's insufficient to
        // rely on `visited` alone here, because we may end up removing a directory whose parent
        // directory doesn't contain any files, leaving the _parent_ directory empty.
        for path in path
            .ancestors()
            .take_while(|&path| path != canonical_site_packages)
        {
            // If the directory contains a `__pycache__` directory, always remove it. `__pycache__`
            // may or may not be listed in the RECORD, but installers are expected to be smart
            // enough to remove it either way.
            let pycache = path.join("__pycache__");
            match fs_err::remove_dir_all(&pycache) {
                Ok(()) => uninstall.record_removed_directory(&pycache),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }

            // Close the directory handle before removing an empty directory.
            match fs_err::read_dir(path) {
                Ok(mut entries) => {
                    if entries.next().is_some() {
                        break;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
                Err(err) => return Err(err.into()),
            }

            fs_err::remove_dir(path)?;

            uninstall.record_removed_directory(path);
        }
    }

    Ok(uninstall)
}

static WARNED_FOR_RECORD_ENTRY_PACKAGE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static WARNED_FOR_EGG_TOP_LEVEL_PACKAGE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn should_warn_for_distribution(
    warned: &OnceLock<Mutex<HashSet<String>>>,
    distribution: &impl Display,
) -> bool {
    warned
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .expect("The mutex is broken, did some other thread panic?")
        .insert(distribution.to_string())
}

struct RecordPathResolver<'a> {
    site_packages: &'a Path,
    roots: [InstallationSchemeRoot<'a>; 5],
    /// Cache one parent so processing an ancestor evicts its descendant before deletion.
    cached_parent: Option<ResolvedRecordParent>,
}

impl<'a> RecordPathResolver<'a> {
    fn new(site_packages: &'a Path, layout: &'a Layout) -> Self {
        Self {
            site_packages,
            roots: installation_scheme_paths(layout).map(|path| InstallationSchemeRoot {
                path,
                resolved: None,
            }),
            cached_parent: None,
        }
    }

    /// Resolve a `RECORD` entry inside the installation scheme, warning otherwise.
    ///
    /// Resolve parent components to prevent an entry from escaping through a symlink. Keep the
    /// final component unresolved so an entry that names a symlink removes the link itself.
    fn resolve(
        &mut self,
        path: &str,
        distribution: impl Display,
    ) -> std::io::Result<Option<PathBuf>> {
        let target = self.site_packages.join(path);
        let normalized = normalize_path(&target);

        // `purelib` or `platlib` are site-packages (depending on `Root-Is-Purelib`). As
        // `.data/*` goes into the directories of `scheme`, `.dist-info` goes into site-packages
        // and all other content goes into site-packages, the condition below covers all valid
        // directories, in venvs, system interpreters and custom installation schemes.
        //
        // For a venv, `data` is the venv root: A wheel can write into the entire venv through
        // `.data/data`. For a system environment, wheels are allowed to write to
        // whole system directories, for example `data` is `/usr/local` for system Python on
        // Ubuntu 24.04.
        if self
            .roots
            .iter()
            .any(|root| normalized.starts_with(root.path))
            && let Some((parent, filename)) = target.parent().zip(target.file_name())
        {
            if let Some(cached) = &self.cached_parent
                && cached.original == parent
            {
                return Ok(Some(cached.resolved.join(filename)));
            }
            self.cached_parent = None;
            let resolved_parent = match canonicalize_directory(parent) {
                Ok(parent) => parent,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(err) => return Err(err),
            };
            let resolved = resolved_parent.join(filename);
            let mut first_error = None;
            for root in &mut self.roots {
                match root.resolve() {
                    Ok(root) if resolved.starts_with(root) => {
                        // A target can be a scheme root even when its parent is outside the scheme.
                        if resolved_parent.starts_with(root) {
                            self.cached_parent = Some(ResolvedRecordParent {
                                original: parent.to_path_buf(),
                                resolved: resolved_parent,
                            });
                        }
                        return Ok(Some(resolved));
                    }
                    Ok(_) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => {
                        first_error.get_or_insert(err);
                    }
                }
            }
            if let Some(err) = first_error {
                return Err(err);
            }
        }

        // A package that does this is malformed to the point of being a risk to the user, be
        // annoying about it, but only once per package.
        if should_warn_for_distribution(&WARNED_FOR_RECORD_ENTRY_PACKAGE, &distribution) {
            warn_user!(
                "Invalid `RECORD` entry in `{}` that escapes the Python environment, skipping: {}",
                distribution,
                path
            );
        }
        Ok(None)
    }
}

/// A parent directory validated inside the scheme for consecutive `RECORD` entries.
struct ResolvedRecordParent {
    original: PathBuf,
    resolved: PathBuf,
}

struct InstallationSchemeRoot<'a> {
    path: &'a Path,
    resolved: Option<PathBuf>,
}

impl InstallationSchemeRoot<'_> {
    fn resolve(&mut self) -> std::io::Result<&Path> {
        let resolved = match &mut self.resolved {
            Some(resolved) => resolved,
            slot @ None => slot.insert(canonicalize_directory(self.path)?),
        };
        Ok(resolved)
    }
}

/// Resolve a directory, treating the empty parent of a relative path as the working directory.
fn canonicalize_directory(path: &Path) -> std::io::Result<PathBuf> {
    fs_err::canonicalize(if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    })
}

/// Return whether deleting a directory would remove an installation-scheme root or its contents.
fn contains_installation_scheme(path: &Path, layout: &Layout) -> std::io::Result<bool> {
    let target = fs_err::canonicalize(path)?;
    for root in installation_scheme_paths(layout) {
        // An ancestor can contain a symlink to a scheme root even when the root resolves elsewhere.
        for ancestor in root.ancestors() {
            match fs_err::canonicalize(ancestor) {
                Ok(ancestor) if ancestor.starts_with(&target) => return Ok(true),
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
    }
    Ok(false)
}

fn installation_scheme_paths(layout: &Layout) -> [&Path; 5] {
    [
        &layout.scheme.data,
        &layout.scheme.purelib,
        &layout.scheme.platlib,
        &layout.scheme.scripts,
        &layout.scheme.include,
    ]
}

/// Iterate over trimmed, nonempty lines in legacy installation metadata.
fn trimmed_nonempty_lines(contents: &str) -> impl Iterator<Item = &str> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
}

/// Check that a `top_level.txt` entry names a single top-level module or package.
///
/// Unlike wheel `RECORD` entries, egg `top_level.txt` entries refer to direct children of the
/// egg's base location, not arbitrary paths. Treating them as paths can make uninstall delete
/// directories outside `site-packages`.
fn is_valid_top_level_entry(entry: &str, distribution: impl Display) -> bool {
    if entry.parse::<Identifier>().is_ok() {
        true
    } else {
        if should_warn_for_distribution(&WARNED_FOR_EGG_TOP_LEVEL_PACKAGE, &distribution) {
            warn_user!(
                "Invalid `top_level.txt` entry in {} that is not a top-level module or package, skipping: {}",
                distribution,
                entry
            );
        }
        false
    }
}

/// Uninstall the egg represented by the `.egg-info` directory.
///
/// See: <https://github.com/pypa/pip/blob/41587f5e0017bcd849f42b314dc8a34a7db75621/src/pip/_internal/req/req_uninstall.py#L483>
pub fn uninstall_egg(egg_info: &Path, distribution: impl Display) -> Result<Uninstall, Error> {
    let mut uninstall = Uninstall::default();

    let dist_location = egg_info
        .parent()
        .expect("egg-info directory is not in a site-packages directory");

    // Read the `namespace_packages.txt` file, skipping empty or whitespace-only entries.
    let namespace_packages = match fs_err::read_to_string(egg_info.join("namespace_packages.txt")) {
        Ok(namespace_packages) => namespace_packages,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err.into()),
    };
    let namespace_packages = trimmed_nonempty_lines(&namespace_packages).collect::<HashSet<_>>();

    // Read the `top_level.txt` file.
    let top_level_path = egg_info.join("top_level.txt");
    let top_level = match fs_err::read_to_string(&top_level_path) {
        Ok(top_level) => top_level,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::MissingTopLevel(top_level_path));
        }
        Err(err) => return Err(err.into()),
    };

    // Remove everything in `top_level.txt` except namespace packages. Legacy setuptools can write
    // a blank file; joining an empty entry to `dist_location` would target site-packages itself.
    for entry in trimmed_nonempty_lines(&top_level) {
        if namespace_packages.contains(entry) {
            continue;
        }
        if !is_valid_top_level_entry(entry, &distribution) {
            continue;
        }

        let path = dist_location.join(entry);

        // Remove as a directory.
        match fs_err::remove_dir_all(&path) {
            Ok(()) => {
                uninstall.record_removed_directory(&path);
                continue;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        // Remove as a `.py`, `.pyc`, or `.pyo` file.
        for extension in &["py", "pyc", "pyo"] {
            let path = path.with_extension(extension);
            match fs_err::remove_file(&path) {
                Ok(()) => {
                    uninstall.record_removed_file(&path);
                    break;
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
    }

    // Remove the `.egg-info` directory.
    match fs_err::remove_dir_all(egg_info) {
        Ok(()) => uninstall.record_removed_directory(egg_info),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err.into());
        }
    }

    Ok(uninstall)
}

fn normcase(s: &str) -> Cow<'_, str> {
    if cfg!(windows) {
        Cow::Owned(s.replace('/', "\\").to_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

static EASY_INSTALL_PTH: Mutex<()> = Mutex::new(());

/// Uninstall the legacy editable represented by the `.egg-link` file.
///
/// See: <https://github.com/pypa/pip/blob/41587f5e0017bcd849f42b314dc8a34a7db75621/src/pip/_internal/req/req_uninstall.py#L534-L552>
pub fn uninstall_legacy_editable(egg_link: &Path) -> Result<Uninstall, Error> {
    let mut uninstall = Uninstall::default();

    // Find the target line in the `.egg-link` file.
    let contents = fs_err::read_to_string(egg_link)?;
    let target_line = trimmed_nonempty_lines(&contents)
        .next()
        .ok_or_else(|| Error::InvalidEggLink(egg_link.to_path_buf()))?;

    // This comes from `pkg_resources.normalize_path`
    let target_line = normcase(target_line);

    match fs_err::remove_file(egg_link) {
        Ok(()) => uninstall.record_removed_file(egg_link),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    let site_package = egg_link
        .parent()
        .ok_or_else(|| Error::BrokenVenv("`.egg-link` file is not in a directory".to_string()))?;
    let easy_install = site_package.join("easy-install.pth");

    // Since uv has an environment lock, it's enough to add a mutex here to ensure we never
    // lose writes to `easy-install.pth` (this is the only place in uv where `easy-install.pth`
    // is modified).
    let _guard = EASY_INSTALL_PTH.lock().unwrap();

    let content = fs_err::read_to_string(&easy_install)?;
    // https://github.com/pypa/pip/blob/41587f5e0017bcd849f42b314dc8a34a7db75621/src/pip/_internal/req/req_uninstall.py#L634
    let mut lines = content.lines();
    let Some(target_index) = lines.position(|line| normcase(line.trim()) == target_line) else {
        return Ok(uninstall);
    };

    // Retain all lines before and after the first match.
    let mut new_content = String::with_capacity(content.len());
    for line in content.lines().take(target_index).chain(lines) {
        new_content.push_str(line);
        new_content.push('\n');
    }
    write_atomic_sync(&easy_install, new_content)?;
    trace!("Removed line from `easy-install.pth`: {target_line}");

    Ok(uninstall)
}

#[derive(Debug, Default)]
pub struct Uninstall {
    /// The number of files that were removed during the uninstallation.
    pub file_count: usize,
    /// The number of directories that were removed during the uninstallation.
    pub dir_count: usize,
}

impl Uninstall {
    /// Record a removed wheel file and retain its resolved parent for empty-directory cleanup.
    fn record_removed_wheel_file(&mut self, mut path: PathBuf, visited: &mut BTreeSet<PathBuf>) {
        self.record_removed_file(&path);
        if path.pop() {
            visited.insert(path);
        }
    }

    fn record_removed_file(&mut self, path: &Path) {
        trace!("Removed file: {}", path.display());
        self.file_count += 1;
    }

    fn record_removed_directory(&mut self, path: &Path) {
        trace!("Removed directory: {}", path.display());
        self.dir_count += 1;
    }
}

/// Normalize a path, removing things like `.` and `..`.
///
/// Source: <https://github.com/rust-lang/cargo/blob/b48c41aedbd69ee3990d62a0e2006edbb506a480/crates/cargo-util/src/paths.rs#L76C1-L109C2>
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use assert_fs::prelude::*;

    use uv_pypi_types::Scheme;

    use crate::Layout;
    use crate::uninstall::{is_valid_top_level_entry, uninstall_egg, uninstall_wheel};

    #[cfg(windows)]
    #[test]
    fn test_uninstall_legacy_editable_with_mixed_case_path() -> anyhow::Result<()> {
        let temp = assert_fs::TempDir::new()?;
        let site_packages = temp.child("site-packages");
        let project = temp.child("MixedCaseProject");
        project.create_dir_all()?;
        let target = project.path().to_str().expect("valid temporary path");
        let egg_link = site_packages.child("example.egg-link");
        egg_link.write_str(target)?;
        let easy_install = site_packages.child("easy-install.pth");
        easy_install.write_str(&format!("other\n{target}\nretained\n"))?;

        super::uninstall_legacy_editable(egg_link.path())?;

        assert_eq!(
            fs_err::read_to_string(easy_install.path())?,
            "other\nretained\n"
        );
        assert!(!egg_link.exists());
        Ok(())
    }

    #[test]
    fn test_top_level_entry_safe_name() {
        let is_valid = |entry| is_valid_top_level_entry(entry, "package");

        assert!(is_valid("package"));
        assert!(is_valid("_package2"));

        assert!(!is_valid(""));
        assert!(!is_valid("."));
        assert!(!is_valid(".."));
        assert!(!is_valid("1package"));
        assert!(!is_valid("package-name"));
        assert!(!is_valid("package.name"));
        assert!(!is_valid("../package"));
        assert!(!is_valid("package/name"));
        assert!(!is_valid(r"package\name"));
        assert!(!is_valid("C:target"));
        assert!(!is_valid("C:."));
        assert!(!is_valid("C:.."));
    }

    /// Uninstall must not remove files outside the install scheme.
    #[test]
    fn test_uninstall_record_path_traversal() {
        let venv = assert_fs::TempDir::new().unwrap();
        let site_packages = venv.child("lib/python3.12/site-packages");
        let outside_dir = assert_fs::TempDir::new().unwrap();

        // Create a file outside site-packages that a malicious RECORD might target.
        let target_file = outside_dir.child("traversal_target.txt");
        target_file.write_str("I should not be deleted").unwrap();

        // Build a relative traversal path from site-packages to the target file.
        let dist_info = site_packages.child("evilpkg-0.1.0.dist-info");
        dist_info.create_dir_all().unwrap();
        let target_path = pathdiff::diff_paths(target_file.path(), site_packages.path()).unwrap();
        assert!(site_packages.join(&target_path).exists());

        // Add the invalid path to the RECORD.
        let record_content = format!(
            "evilpkg/__init__.py,,0\n\
             evilpkg-0.1.0.dist-info/METADATA,,0\n\
             evilpkg-0.1.0.dist-info/RECORD,,\n\
             {},,0\n",
            target_path.display()
        );
        dist_info
            .child("RECORD")
            .write_str(&record_content)
            .unwrap();

        // Also create the legitimate files so uninstall can remove them.
        let init_py = site_packages.child("evilpkg/__init__.py");
        init_py.touch().unwrap();
        let metadata = dist_info.child("METADATA");
        metadata.touch().unwrap();

        // Something that looks sufficiently like a Unix environment.
        let layout = Layout {
            sys_executable: venv.path().join("bin/python"),
            python_version: (3, 13),
            os_name: "posix".to_string(),
            scheme: Scheme {
                purelib: site_packages.to_path_buf(),
                platlib: site_packages.to_path_buf(),
                scripts: venv.path().join("bin"),
                data: venv.path().to_path_buf(),
                include: venv.path().join("include/python3.12"),
            },
        };

        uninstall_wheel(dist_info.path(), "evilpkg 0.1.0", &layout).unwrap();

        // The regular package files have been removed, while the file outside the scheme still
        // exists.
        assert!(target_file.exists());
        assert!(!metadata.exists());
        assert!(!init_py.exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_uninstall_record_symlink_containment() -> anyhow::Result<()> {
        let temp = assert_fs::TempDir::new()?;
        let venv = temp.child("venv");
        venv.child("lib/python3.12").create_dir_all()?;
        let site_packages = venv.child("lib/python3.12/site-packages");
        let actual_site_packages = temp.child("actual-site-packages");
        actual_site_packages.create_dir_all()?;
        std::os::unix::fs::symlink(actual_site_packages.path(), site_packages.path())?;

        let outside = temp.child("outside");
        let sentinel = outside.child("sentinel.txt");
        sentinel.write_str("keep")?;
        std::os::unix::fs::symlink(outside.path(), site_packages.child("escape").path())?;
        let final_symlink = site_packages.child("final-symlink");
        std::os::unix::fs::symlink(sentinel.path(), final_symlink.path())?;

        let shared = actual_site_packages.child("shared");
        let first = shared.child("first.txt");
        let second = shared.child("second.txt");
        first.touch()?;
        second.touch()?;
        let alias = site_packages.child("alias");
        std::os::unix::fs::symlink(shared.path(), alias.path())?;

        let dist_info = site_packages.child("example-1.0.dist-info");
        dist_info.child("RECORD").write_str(concat!(
            "escape/sentinel.txt,,\n",
            "alias/first.txt,,\n",
            "alias,,\n",
            "alias/second.txt,,\n",
            "final-symlink,,\n",
            "example-1.0.dist-info/RECORD,,\n",
        ))?;
        let layout = Layout {
            sys_executable: venv.path().join("bin/python"),
            python_version: (3, 12),
            os_name: "posix".to_string(),
            scheme: Scheme {
                purelib: site_packages.to_path_buf(),
                platlib: site_packages.to_path_buf(),
                scripts: venv.path().join("bin"),
                data: venv.path().to_path_buf(),
                include: venv.path().join("include"),
            },
        };

        let result = uninstall_wheel(dist_info.path(), "example 1.0", &layout);
        assert!(sentinel.exists());
        result?;
        assert!(!first.exists());
        assert!(second.exists());
        assert!(!alias.exists());
        assert!(!final_symlink.exists());
        assert!(!dist_info.exists());
        Ok(())
    }

    /// A directory can lie within the data scheme while containing another scheme's root.
    #[test]
    fn test_uninstall_record_preserves_installation_scheme() {
        let venv = assert_fs::TempDir::new().unwrap();
        let site_packages = venv.child("lib/python3.12/site-packages");
        let dist_info = site_packages.child("evilpkg-0.1.0.dist-info");
        dist_info.create_dir_all().unwrap();
        dist_info
            .child("RECORD")
            .write_str("../../../lib,,\nevilpkg,,\nevilpkg-0.1.0.dist-info/RECORD,,\n")
            .unwrap();
        let sibling = site_packages.child("sibling/__init__.py");
        sibling.touch().unwrap();
        let package = site_packages.child("evilpkg");
        package.child("__init__.py").touch().unwrap();

        let layout = Layout {
            sys_executable: venv.path().join("bin/python"),
            python_version: (3, 12),
            os_name: "posix".to_string(),
            scheme: Scheme {
                purelib: site_packages.to_path_buf(),
                platlib: site_packages.to_path_buf(),
                scripts: venv.path().join("bin"),
                data: venv.path().to_path_buf(),
                include: venv.path().join("include/python3.12"),
            },
        };

        uninstall_wheel(dist_info.path(), "evilpkg 0.1.0", &layout).unwrap();

        assert!(sibling.exists());
        assert!(!package.exists());
        assert!(!dist_info.exists());
    }

    #[test]
    fn test_uninstall_egg_info_path_traversal() {
        let venv = assert_fs::TempDir::new().unwrap();
        let site_packages = venv.child("lib/python3.12/site-packages");

        // Create directories outside site-packages, but inside the environment. Egg uninstall should
        // still reject them, even though wheel RECORD entries may target other install-scheme
        // directories.
        let target_dir = venv.child("traversal_target");
        let target_file = target_dir.child("secret.txt");
        target_file.write_str("I should not be deleted").unwrap();
        // Build a relative traversal path from site-packages to the target directory.
        let egg_info = site_packages.child("evilpkg-0.1.0.egg-info");
        egg_info.create_dir_all().unwrap();
        let target_path = pathdiff::diff_paths(target_dir.path(), site_packages.path()).unwrap();
        assert!(site_packages.join(&target_path).exists());

        // Create a fake egg-info directory with a path traversal entry in `top_level.txt`.
        egg_info
            .child("top_level.txt")
            .write_str(&format!("evilpkg\n{}\n", target_path.display()))
            .unwrap();

        // Also create the legitimate package directory so uninstall can remove it.
        let init_py = site_packages.child("evilpkg").child("__init__.py");
        init_py.touch().unwrap();

        uninstall_egg(egg_info.path(), "evilpkg 0.1.0").unwrap();

        // The regular package directory has been removed, while the directory outside
        // site-packages still exists.
        assert!(target_dir.exists());
        assert!(target_file.exists());
        assert!(!init_py.exists());
        assert!(!egg_info.exists());
    }

    /// Regression test for <https://github.com/astral-sh/uv/issues/19113>.
    ///
    /// Legacy setuptools writes a `top_level.txt` that contains just a newline when the
    /// distribution has no top-level modules. Previously, [`uninstall_egg`] parsed that as a
    /// single empty entry, joined it onto `site-packages`, and called `remove_dir_all` on the
    /// result, wiping out every other package in the environment. Uninstalling such a package
    /// must leave its siblings untouched.
    #[test]
    fn test_uninstall_egg_info_empty_top_level() {
        let venv = assert_fs::TempDir::new().unwrap();
        let site_packages = venv.child("lib/python3.12/site-packages");
        site_packages.create_dir_all().unwrap();

        // A sibling package that must survive the uninstall.
        let sibling_init = site_packages.child("sibling").child("__init__.py");
        sibling_init.touch().unwrap();
        let sibling_dist_info = site_packages.child("sibling-1.0.0.dist-info");
        sibling_dist_info.create_dir_all().unwrap();

        // The egg-info for the package we're uninstalling, with a `top_level.txt` that
        // contains only a newline (as legacy setuptools writes for an empty package).
        let egg_info = site_packages.child("emptypkg-0.1.0.egg-info");
        egg_info.create_dir_all().unwrap();
        egg_info.child("top_level.txt").write_str("\n").unwrap();

        uninstall_egg(egg_info.path(), "emptypkg 0.1.0").unwrap();

        // The egg-info is gone, but the rest of site-packages (including the sibling
        // package) survives.
        assert!(!egg_info.exists());
        assert!(
            site_packages.exists(),
            "uninstall must not remove site-packages itself"
        );
        assert!(sibling_init.exists(), "sibling package must not be removed");
        assert!(
            sibling_dist_info.exists(),
            "sibling dist-info must not be removed"
        );
    }

    /// Same bug shape as #19113, but triggered by a blank or whitespace-only line embedded
    /// between valid entries in `top_level.txt`. Exercises the filter in combination with
    /// real entries to make sure they're still honored after skipping empties.
    #[test]
    fn test_uninstall_egg_info_blank_lines_in_top_level() {
        let venv = assert_fs::TempDir::new().unwrap();
        let site_packages = venv.child("lib/python3.12/site-packages");
        site_packages.create_dir_all().unwrap();

        // A sibling package that must survive.
        let sibling_init = site_packages.child("sibling").child("__init__.py");
        sibling_init.touch().unwrap();

        // Two real top-level modules that should be removed.
        let pkg_a_init = site_packages.child("pkg_a").child("__init__.py");
        pkg_a_init.touch().unwrap();
        let pkg_b_init = site_packages.child("pkg_b").child("__init__.py");
        pkg_b_init.touch().unwrap();

        // `top_level.txt` with a leading blank line, a whitespace-only line between the two
        // valid entries, a trailing blank line, and `\r\n` line endings mixed in.
        let egg_info = site_packages.child("mixedpkg-0.1.0.egg-info");
        egg_info.create_dir_all().unwrap();
        egg_info
            .child("top_level.txt")
            .write_str("\npkg_a\n   \r\npkg_b\n\n")
            .unwrap();

        uninstall_egg(egg_info.path(), "mixedpkg 0.1.0").unwrap();

        // The two named packages are gone, the egg-info is gone, and site-packages plus
        // the sibling survive.
        assert!(!egg_info.exists());
        assert!(!pkg_a_init.exists(), "pkg_a must be removed");
        assert!(!pkg_b_init.exists(), "pkg_b must be removed");
        assert!(
            site_packages.exists(),
            "uninstall must not remove site-packages itself"
        );
        assert!(sibling_init.exists(), "sibling package must not be removed");
    }
}
