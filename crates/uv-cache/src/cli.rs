use std::io;
use std::path::{Path, PathBuf};
use uv_static::EnvVars;

use crate::Cache;
use clap::{Parser, ValueHint};
use tracing::{debug, warn};

#[derive(Parser, Debug, Clone)]
#[command(next_help_heading = "Cache options")]
pub struct CacheArgs {
    /// Avoid reading from or writing to the cache, instead using a temporary directory for the
    /// duration of the operation.
    #[arg(
        global = true,
        long,
        short,
        alias = "no-cache-dir",
        env = EnvVars::UV_NO_CACHE,
        value_parser = clap::builder::BoolishValueParser::new(),
    )]
    pub no_cache: bool,

    /// Path to the cache directory.
    ///
    /// Defaults to `$XDG_CACHE_HOME/uv` or `$HOME/.cache/uv` on macOS and Linux, and
    /// `%LOCALAPPDATA%\uv\cache` on Windows.
    ///
    /// To view the location of the cache directory, run `uv cache dir`.
    #[arg(global = true, long, env = EnvVars::UV_CACHE_DIR, value_hint = ValueHint::DirPath)]
    pub cache_dir: Option<PathBuf>,
}

impl Cache {
    /// Prefer, in order:
    ///
    /// 1. A temporary cache directory, if the user requested `--no-cache`.
    /// 2. The specific cache directory specified by the user via `--cache-dir` or `UV_CACHE_DIR`.
    /// 3. The system-appropriate cache directory.
    /// 4. A `.uv_cache` directory in the current working directory.
    ///
    /// Returns an absolute cache dir.
    pub fn from_settings(no_cache: bool, cache_dir: Option<PathBuf>) -> Result<Self, io::Error> {
        if no_cache {
            Self::temp()
        } else if let Some(cache_dir) = cache_dir {
            Ok(Self::from_path(cache_dir))
        } else if let Some(cache_dir) = uv_dirs::legacy_user_cache_dir().filter(|dir| dir.exists())
        {
            // If the user has an existing directory at (e.g.) `/Users/user/Library/Caches/uv`,
            // respect it for backwards compatibility. Otherwise, prefer the XDG strategy, even on
            // macOS.
            Ok(Self::from_path(cache_dir))
        } else if let Some(cache_dir) = uv_dirs::user_cache_dir() {
            if cfg!(windows) {
                // On Windows, we append `cache` to the LocalAppData directory, i.e., prefer
                // `C:\Users\User\AppData\Local\uv\cache` over `C:\Users\User\AppData\Local\uv`.
                //
                // Unfortunately, v0.3.0 and v0.3.1 used the latter, so we need to migrate the cache
                // for those users.
                let destination = cache_dir.join("cache");
                let source = cache_dir;
                if let Err(err) = migrate_windows_cache(&source, &destination) {
                    warn!(
                        "Failed to migrate cache from `{}` to `{}`: {err}",
                        source.display(),
                        destination.display()
                    );
                }

                Ok(Self::from_path(destination))
            } else {
                Ok(Self::from_path(cache_dir))
            }
        } else {
            Ok(Self::from_path(".uv_cache"))
        }
    }
}

impl TryFrom<CacheArgs> for Cache {
    type Error = io::Error;

    fn try_from(value: CacheArgs) -> Result<Self, Self::Error> {
        Self::from_settings(value.no_cache, value.cache_dir)
    }
}

/// Migrate the Windows cache from `C:\Users\User\AppData\Local\uv` to `C:\Users\User\AppData\Local\uv\cache`.
fn migrate_windows_cache(source: &Path, destination: &Path) -> Result<(), io::Error> {
    // The list of expected cache buckets in v0.3.0.
    for directory in [
        "built-wheels-v3",
        "flat-index-v0",
        "git-v0",
        "interpreter-v2",
        "simple-v12",
        "wheels-v1",
        "archive-v0",
        "builds-v0",
        "environments-v1",
    ] {
        let source = source.join(directory);
        let destination = destination.join(directory);

        // Migrate the cache bucket.
        if source.exists() {
            debug!(
                "Migrating cache bucket from {} to {}",
                source.display(),
                destination.display()
            );
            if let Some(parent) = destination.parent() {
                fs_err::create_dir_all(parent)?;
            }
            fs_err::rename(&source, &destination)?;
        }
    }

    // The list of expected cache files in v0.3.0.
    for file in [".gitignore", "CACHEDIR.TAG"] {
        let source = source.join(file);
        let destination = destination.join(file);

        // Migrate the cache file.
        if source.exists() {
            debug!(
                "Migrating cache file from {} to {}",
                source.display(),
                destination.display()
            );
            if let Some(parent) = destination.parent() {
                fs_err::create_dir_all(parent)?;
            }
            fs_err::rename(&source, &destination)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::migrate_windows_cache;

    #[test]
    fn legacy_windows_cache_migrates_expected_entries() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("uv");
        let destination = source.join("cache");

        // These are the v0.3.0 names, not the current cache bucket versions.
        let directories = [
            "built-wheels-v3",
            "flat-index-v0",
            "git-v0",
            "interpreter-v2",
            "simple-v12",
            "wheels-v1",
            "archive-v0",
            "builds-v0",
            "environments-v1",
        ];
        let files = [(".gitignore", "*"), ("CACHEDIR.TAG", "legacy cache tag")];
        for directory in directories {
            let path = source.join(directory);
            fs_err::create_dir_all(&path)?;
            fs_err::write(path.join("payload"), directory)?;
        }
        for (file, contents) in files {
            fs_err::write(source.join(file), contents)?;
        }
        let unrelated_directory = source.join("python");
        fs_err::create_dir(&unrelated_directory)?;
        fs_err::write(unrelated_directory.join("payload"), "keep directory")?;
        fs_err::write(source.join("unrelated.txt"), "keep file")?;
        assert!(!destination.exists());

        // A second migration must leave the already-moved entries unchanged.
        for _ in 0..2 {
            migrate_windows_cache(&source, &destination)?;

            for directory in directories {
                assert!(!source.join(directory).exists());
                assert_eq!(
                    fs_err::read_to_string(destination.join(directory).join("payload"))?,
                    directory
                );
            }
            for (file, contents) in files {
                assert!(!source.join(file).exists());
                assert_eq!(fs_err::read_to_string(destination.join(file))?, contents);
            }
            assert_eq!(
                fs_err::read_dir(&destination)?
                    .collect::<io::Result<Vec<_>>>()?
                    .len(),
                directories.len() + files.len()
            );
            assert_eq!(
                fs_err::read_to_string(unrelated_directory.join("payload"))?,
                "keep directory"
            );
            assert_eq!(
                fs_err::read_to_string(source.join("unrelated.txt"))?,
                "keep file"
            );
        }
        Ok(())
    }

    #[test]
    fn legacy_windows_cache_migrates_base_file_without_buckets() -> io::Result<()> {
        let root = tempfile::tempdir()?;
        let source = root.path().join("uv");
        let destination = source.join("cache");

        migrate_windows_cache(&source, &destination)?;
        assert!(!source.exists());
        assert!(!destination.exists());

        fs_err::create_dir(&source)?;
        fs_err::write(source.join("CACHEDIR.TAG"), "legacy cache tag")?;
        migrate_windows_cache(&source, &destination)?;

        assert!(!source.join("CACHEDIR.TAG").exists());
        assert_eq!(
            fs_err::read_to_string(destination.join("CACHEDIR.TAG"))?,
            "legacy cache tag"
        );
        assert_eq!(
            fs_err::read_dir(&destination)?
                .collect::<io::Result<Vec<_>>>()?
                .len(),
            1
        );
        Ok(())
    }
}
