use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::{debug, info_span, warn};

use uv_fs::{Simplified, created_time};
use uv_normalize::PackageName;

use crate::git_info::{Commit, Tags};
use crate::glob::cluster_globs;
use crate::timestamp::Timestamp;

#[derive(Debug, thiserror::Error)]
pub enum CacheInfoError {
    #[error("Failed to parse glob patterns for `cache-keys`: {0}")]
    Glob(#[from] globwalk::GlobError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(
        "Package `{package}` referenced by `cache-keys` was not found in the workspace containing `{}`",
        directory.user_display()
    )]
    PackageNotFound {
        package: PackageName,
        directory: PathBuf,
    },
    #[error("Package cache key cycle detected: {0}")]
    PackageCycle(String),
}

/// The information used to determine whether a built distribution is up-to-date, based on the
/// timestamps of relevant files, the current commit of a repository, etc.
#[derive(Default, Debug, Clone, Hash, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct CacheInfo {
    /// The timestamp of the most recent `ctime` of any relevant files, at the time of the build.
    /// The timestamp will typically be the maximum of the `ctime` values of the `pyproject.toml`,
    /// `setup.py`, and `setup.cfg` files, if they exist; however, users can provide additional
    /// files to timestamp via the `cache-keys` field.
    timestamp: Option<Timestamp>,
    /// The commit at which the distribution was built.
    commit: Option<Commit>,
    /// The Git tags present at the time of the build.
    tags: Option<Tags>,
    /// Environment variables to include in the cache key.
    #[serde(default)]
    env: BTreeMap<String, Option<String>>,
    /// The timestamp or inode of any directories that should be considered in the cache key.
    #[serde(default)]
    directories: BTreeMap<Cow<'static, str>, Option<DirectoryTimestamp>>,
    /// The evaluated cache information for referenced workspace packages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    packages: BTreeMap<PackageName, Option<PackageCacheInfo>>,
}

/// The location and evaluated cache information for a referenced workspace package.
#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct PackageCacheInfo {
    /// The path to the package, relative to the project that references it.
    root: PathBuf,
    /// The evaluated cache information for the package.
    cache_info: Box<CacheInfo>,
}

impl CacheInfo {
    /// Return the [`CacheInfo`] for a given timestamp.
    pub fn from_timestamp(timestamp: Timestamp) -> Self {
        Self {
            timestamp: Some(timestamp),
            ..Self::default()
        }
    }

    /// Compute the cache info for a given path, which may be a file or a directory.
    pub fn from_path(path: &Path) -> Result<Self, CacheInfoError> {
        let metadata = fs_err::metadata(path)?;
        if metadata.is_file() {
            Ok(Self::from_file(path)?)
        } else {
            Self::from_directory(path)
        }
    }

    /// Compute the cache info for a given directory.
    pub fn from_directory(directory: &Path) -> Result<Self, CacheInfoError> {
        Self::from_directory_inner(directory, None, &mut Vec::new())
    }

    /// Compute the cache info for a given directory, resolving package cache keys against the
    /// provided workspace members.
    pub fn from_directory_with_packages(
        directory: &Path,
        packages: &BTreeMap<PackageName, PathBuf>,
    ) -> Result<Self, CacheInfoError> {
        Self::from_directory_inner(directory, Some(packages), &mut Vec::new())
    }

    /// Recompute the cache info using package locations from the previous evaluation.
    pub fn refresh_from_path(&self, path: &Path) -> Result<Self, CacheInfoError> {
        let metadata = fs_err::metadata(path)?;
        if metadata.is_file() {
            Ok(Self::from_file(path)?)
        } else {
            self.refresh_from_directory(path)
        }
    }

    /// Recompute the cache info for a directory using package locations from the previous
    /// evaluation.
    pub fn refresh_from_directory(&self, directory: &Path) -> Result<Self, CacheInfoError> {
        Self::from_directory_with_previous(directory, self, &mut Vec::new())
    }

    /// Return whether the project declares a package cache key.
    pub fn has_package_cache_key(directory: &Path) -> bool {
        Self::read_cache_keys(directory).is_some_and(|cache_keys| {
            cache_keys
                .iter()
                .any(|key| matches!(key, CacheKey::Package { .. }))
        })
    }

    fn read_cache_keys(directory: &Path) -> Option<Vec<CacheKey>> {
        let pyproject_path = directory.join("pyproject.toml");
        let contents = fs_err::read_to_string(&pyproject_path).ok()?;
        let result = info_span!("toml::from_str cache keys", path = %pyproject_path.display())
            .in_scope(|| toml::from_str::<PyProjectToml>(&contents));
        result
            .ok()?
            .tool
            .and_then(|tool| tool.uv)
            .and_then(|tool_uv| tool_uv.cache_keys)
    }

    fn from_directory_inner(
        directory: &Path,
        package_roots: Option<&BTreeMap<PackageName, PathBuf>>,
        stack: &mut Vec<PackageName>,
    ) -> Result<Self, CacheInfoError> {
        Self::from_directory_impl(directory, stack, |package, stack| {
            let Some(package_roots) = package_roots else {
                return Ok(None);
            };
            let Some(root) = package_roots.get(package) else {
                return Err(CacheInfoError::PackageNotFound {
                    package: package.clone(),
                    directory: directory.to_path_buf(),
                });
            };
            let relative_root = pathdiff::diff_paths(root, directory).ok_or_else(|| {
                CacheInfoError::PackageNotFound {
                    package: package.clone(),
                    directory: directory.to_path_buf(),
                }
            })?;
            let cache_info = Self::from_directory_inner(root, Some(package_roots), stack)?;
            Ok(Some(PackageCacheInfo {
                root: relative_root,
                cache_info: Box::new(cache_info),
            }))
        })
    }

    fn from_directory_with_previous(
        directory: &Path,
        previous: &Self,
        stack: &mut Vec<PackageName>,
    ) -> Result<Self, CacheInfoError> {
        Self::from_directory_impl(directory, stack, |package, stack| {
            let Some(Some(previous_package)) = previous.packages.get(package) else {
                return Ok(None);
            };
            let root = directory.join(&previous_package.root);
            let cache_info =
                Self::from_directory_with_previous(&root, &previous_package.cache_info, stack)?;
            Ok(Some(PackageCacheInfo {
                root: previous_package.root.clone(),
                cache_info: Box::new(cache_info),
            }))
        })
    }

    fn from_directory_impl<F>(
        directory: &Path,
        stack: &mut Vec<PackageName>,
        mut resolve_package: F,
    ) -> Result<Self, CacheInfoError>
    where
        F: FnMut(
            &PackageName,
            &mut Vec<PackageName>,
        ) -> Result<Option<PackageCacheInfo>, CacheInfoError>,
    {
        let mut commit = None;
        let mut tags = None;
        let mut last_changed: Option<(PathBuf, Timestamp)> = None;
        let mut directories = BTreeMap::new();
        let mut env = BTreeMap::new();
        let mut packages = BTreeMap::new();

        // Read the cache keys.
        let cache_keys = Self::read_cache_keys(directory);

        // If no cache keys were defined, use the defaults.
        let cache_keys = cache_keys.unwrap_or_else(|| {
            vec![
                CacheKey::Path(Cow::Borrowed("pyproject.toml")),
                CacheKey::Path(Cow::Borrowed("setup.py")),
                CacheKey::Path(Cow::Borrowed("setup.cfg")),
                CacheKey::Directory {
                    dir: Cow::Borrowed("src"),
                },
            ]
        });

        // Incorporate timestamps from any direct filepaths.
        let mut globs = vec![];
        for cache_key in cache_keys {
            match cache_key {
                CacheKey::Path(file) | CacheKey::File { file } => {
                    if file
                        .as_ref()
                        .chars()
                        .any(|c| matches!(c, '*' | '?' | '[' | '{'))
                    {
                        // Defer globs to a separate pass.
                        globs.push(file);
                        continue;
                    }

                    // Treat the path as a file.
                    let path = directory.join(file.as_ref());
                    let metadata = match path.metadata() {
                        Ok(metadata) => metadata,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                            continue;
                        }
                        Err(err) => {
                            warn!("Failed to read metadata for file: {err}");
                            continue;
                        }
                    };
                    if !metadata.is_file() {
                        warn!(
                            "Expected file for cache key, but found directory: `{}`",
                            path.display()
                        );
                        continue;
                    }
                    let timestamp = Timestamp::from_metadata(&metadata);
                    if last_changed.as_ref().is_none_or(|(_, prev_timestamp)| {
                        *prev_timestamp < Timestamp::from_metadata(&metadata)
                    }) {
                        last_changed = Some((path, timestamp));
                    }
                }
                CacheKey::Directory { dir } => {
                    // Treat the path as a directory.
                    let path = directory.join(dir.as_ref());
                    let metadata = match path.metadata() {
                        Ok(metadata) => metadata,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                            directories.insert(dir, None);
                            continue;
                        }
                        Err(err) => {
                            warn!("Failed to read metadata for directory: {err}");
                            continue;
                        }
                    };
                    if !metadata.is_dir() {
                        warn!(
                            "Expected directory for cache key, but found file: `{}`",
                            path.display()
                        );
                        continue;
                    }

                    if let Ok(created) = created_time(&path, &metadata) {
                        // Prefer the creation time.
                        directories.insert(
                            dir,
                            Some(DirectoryTimestamp::Timestamp(Timestamp::from(created))),
                        );
                    } else {
                        // Fall back to the inode.
                        cfg_select! {
                            unix => {
                                use std::os::unix::fs::MetadataExt;
                                directories.insert(
                                    dir,
                                    Some(DirectoryTimestamp::Inode(metadata.ino())),
                                );
                            },
                            _ => {
                                warn!(
                                    "Failed to read creation time for directory: `{}`",
                                    path.display()
                                );
                            },
                        }
                    }
                }
                CacheKey::Git {
                    git: GitPattern::Bool(true),
                } => match Commit::from_repository(directory) {
                    Ok(commit_info) => commit = Some(commit_info),
                    Err(err) => {
                        debug!("Failed to read the current commit: {err}");
                    }
                },
                CacheKey::Git {
                    git: GitPattern::Set(set),
                } => {
                    if set.commit.unwrap_or(false) {
                        match Commit::from_repository(directory) {
                            Ok(commit_info) => commit = Some(commit_info),
                            Err(err) => {
                                debug!("Failed to read the current commit: {err}");
                            }
                        }
                    }
                    if set.tags.unwrap_or(false) {
                        match Tags::from_repository(directory) {
                            Ok(tags_info) => tags = Some(tags_info),
                            Err(err) => {
                                debug!("Failed to read the current tags: {err}");
                            }
                        }
                    }
                }
                CacheKey::Git {
                    git: GitPattern::Bool(false),
                } => {}
                CacheKey::Environment { env: var } => {
                    let value = std::env::var(&var).ok();
                    env.insert(var, value);
                }
                CacheKey::Package { package } => {
                    if let Some(position) = stack.iter().position(|item| item == &package) {
                        let cycle = stack[position..]
                            .iter()
                            .chain(std::iter::once(&package))
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" -> ");
                        return Err(CacheInfoError::PackageCycle(cycle));
                    }
                    stack.push(package.clone());
                    let cache_info = resolve_package(&package, stack)?;
                    stack.pop();
                    packages.insert(package, cache_info);
                }
            }
        }

        // If we have any globs, first cluster them using LCP and then do a single pass on each group.
        if !globs.is_empty() {
            for (glob_base, glob_patterns) in cluster_globs(&globs) {
                let walker = globwalk::GlobWalkerBuilder::from_patterns(
                    directory.join(glob_base),
                    &glob_patterns,
                )
                .file_type(globwalk::FileType::FILE | globwalk::FileType::SYMLINK)
                .build()?;
                for entry in walker {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(err) => {
                            warn!("Failed to read glob entry: {err}");
                            continue;
                        }
                    };
                    let metadata = if entry.path_is_symlink() {
                        // resolve symlinks for leaf entries without following symlinks while globbing
                        match fs_err::metadata(entry.path()) {
                            Ok(metadata) => metadata,
                            Err(err) => {
                                warn!("Failed to resolve symlink for glob entry: {err}");
                                continue;
                            }
                        }
                    } else {
                        match entry.metadata() {
                            Ok(metadata) => metadata,
                            Err(err) => {
                                warn!("Failed to read metadata for glob entry: {err}");
                                continue;
                            }
                        }
                    };
                    if !metadata.is_file() {
                        if !entry.path_is_symlink() {
                            // don't warn if it was a symlink - it may legitimately resolve to a directory
                            warn!(
                                "Expected file for cache key, but found directory: `{}`",
                                entry.path().display()
                            );
                        }
                        continue;
                    }
                    let timestamp = Timestamp::from_metadata(&metadata);
                    if last_changed.as_ref().is_none_or(|(_, prev_timestamp)| {
                        *prev_timestamp < Timestamp::from_metadata(&metadata)
                    }) {
                        last_changed = Some((entry.into_path(), timestamp));
                    }
                }
            }
        }

        let timestamp = if let Some((path, timestamp)) = last_changed {
            debug!(
                "Computed cache info: {timestamp:?}, {commit:?}, {tags:?}, {env:?}, {directories:?}. Most recently modified: {}",
                path.user_display()
            );
            Some(timestamp)
        } else {
            None
        };

        Ok(Self {
            timestamp,
            commit,
            tags,
            env,
            directories,
            packages,
        })
    }

    /// Compute the cache info for a given file, assumed to be a binary or source distribution
    /// represented as (e.g.) a `.whl` or `.tar.gz` archive.
    pub fn from_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let metadata = fs_err::metadata(path.as_ref())?;
        let timestamp = Timestamp::from_metadata(&metadata);
        Ok(Self {
            timestamp: Some(timestamp),
            ..Self::default()
        })
    }

    /// Returns `true` if the cache info is empty.
    pub fn is_empty(&self) -> bool {
        self.timestamp.is_none()
            && self.commit.is_none()
            && self.tags.is_none()
            && self.env.is_empty()
            && self.directories.is_empty()
            && self.packages.is_empty()
    }
}

/// A `pyproject.toml` with an (optional) `[tool.uv]` section.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct PyProjectToml {
    tool: Option<Tool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Tool {
    uv: Option<ToolUv>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ToolUv {
    cache_keys: Option<Vec<CacheKey>>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged, rename_all = "kebab-case", deny_unknown_fields)]
pub enum CacheKey {
    /// Ex) `"Cargo.lock"` or `"**/*.toml"`
    Path(Cow<'static, str>),
    /// Ex) `{ file = "Cargo.lock" }` or `{ file = "**/*.toml" }`
    File { file: Cow<'static, str> },
    /// Ex) `{ dir = "src" }`
    Directory { dir: Cow<'static, str> },
    /// Ex) `{ git = true }` or `{ git = { commit = true, tags = false } }`
    Git { git: GitPattern },
    /// Ex) `{ env = "UV_CACHE_INFO" }`
    Environment { env: String },
    /// Ex) `{ package = "example" }`
    Package { package: PackageName },
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged, rename_all = "kebab-case", deny_unknown_fields)]
pub enum GitPattern {
    Bool(bool),
    Set(GitSet),
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct GitSet {
    commit: Option<bool>,
    tags: Option<bool>,
}

/// A timestamp used to measure changes to a directory.
#[derive(Debug, Clone, Hash, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(untagged, rename_all = "kebab-case", deny_unknown_fields)]
enum DirectoryTimestamp {
    Timestamp(Timestamp),
    Inode(u64),
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use anyhow::Result;
    use tempfile::TempDir;
    use uv_normalize::PackageName;

    use super::{CacheInfo, CacheInfoError};

    #[test]
    fn package_cache_key() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let foo = temp_dir.path().join("foo");
        let bar = temp_dir.path().join("bar");
        fs_err::create_dir_all(&foo)?;
        fs_err::create_dir_all(&bar)?;
        fs_err::write(
            foo.join("pyproject.toml"),
            r#"
            [tool.uv]
            cache-keys = [{ file = "source.cpp" }]
            "#,
        )?;
        fs_err::write(foo.join("source.cpp"), "first")?;
        fs_err::write(
            bar.join("pyproject.toml"),
            r#"
            [tool.uv]
            cache-keys = [{ package = "foo" }]
            "#,
        )?;

        let package = PackageName::from_str("foo")?;
        let package_roots = BTreeMap::from([(package.clone(), foo.clone())]);
        let initial = CacheInfo::from_directory_with_packages(&bar, &package_roots)?;
        let package_info = initial.packages[&package].as_ref().unwrap();
        assert_eq!(package_info.root, std::path::Path::new("../foo"));

        fs_err::write(foo.join("source.cpp"), "second")?;
        let updated = initial.refresh_from_directory(&bar)?;
        assert_ne!(initial, updated);

        Ok(())
    }

    #[test]
    fn package_cache_key_cycle() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let foo = temp_dir.path().join("foo");
        let bar = temp_dir.path().join("bar");
        fs_err::create_dir_all(&foo)?;
        fs_err::create_dir_all(&bar)?;
        fs_err::write(
            foo.join("pyproject.toml"),
            r#"
            [tool.uv]
            cache-keys = [{ package = "bar" }]
            "#,
        )?;
        fs_err::write(
            bar.join("pyproject.toml"),
            r#"
            [tool.uv]
            cache-keys = [{ package = "foo" }]
            "#,
        )?;

        let package_roots = BTreeMap::from([
            (PackageName::from_str("foo")?, foo),
            (PackageName::from_str("bar")?, bar.clone()),
        ]);
        let err = CacheInfo::from_directory_with_packages(&bar, &package_roots).unwrap_err();
        assert!(
            matches!(err, CacheInfoError::PackageCycle(ref cycle) if cycle == "foo -> bar -> foo")
        );

        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests_unix {
    use anyhow::Result;

    use super::{CacheInfo, Timestamp};

    #[test]
    fn test_cache_info_symlink_resolve() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let dir = dir.path().join("dir");
        fs_err::create_dir_all(&dir)?;

        let write_manifest = |cache_key: &str| {
            fs_err::write(
                dir.join("pyproject.toml"),
                format!(
                    r#"
                [tool.uv]
                cache-keys = [
                    "{cache_key}"
                ]
                "#
                ),
            )
        };

        let touch = |path: &str| -> Result<_> {
            let path = dir.join(path);
            fs_err::create_dir_all(path.parent().unwrap())?;
            fs_err::write(&path, "")?;
            Ok(Timestamp::from_metadata(&path.metadata()?))
        };

        let cache_timestamp = || -> Result<_> { Ok(CacheInfo::from_directory(&dir)?.timestamp) };

        write_manifest("x/**")?;
        assert_eq!(cache_timestamp()?, None);
        let y = touch("x/y")?;
        assert_eq!(cache_timestamp()?, Some(y));
        let z = touch("x/z")?;
        assert_eq!(cache_timestamp()?, Some(z));

        // leaf entry symlink should be resolved
        let a = touch("../a")?;
        fs_err::os::unix::fs::symlink(dir.join("../a"), dir.join("x/a"))?;
        assert_eq!(cache_timestamp()?, Some(a));

        // symlink directories should not be followed while globbing
        let c = touch("../b/c")?;
        fs_err::os::unix::fs::symlink(dir.join("../b"), dir.join("x/b"))?;
        assert_eq!(cache_timestamp()?, Some(a));

        // no globs, should work as expected
        write_manifest("x/y")?;
        assert_eq!(cache_timestamp()?, Some(y));
        write_manifest("x/a")?;
        assert_eq!(cache_timestamp()?, Some(a));
        write_manifest("x/b/c")?;
        assert_eq!(cache_timestamp()?, Some(c));

        // symlink pointing to a directory
        write_manifest("x/*b*")?;
        assert_eq!(cache_timestamp()?, None);

        Ok(())
    }
}
