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
    // Cache buckets and marker files present in v0.3.0.
    const BUCKETS: &[&str] = &[
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
    const FILES: &[&str] = &[".gitignore", "CACHEDIR.TAG"];

    for (kind, entries) in [("bucket", BUCKETS), ("file", FILES)] {
        for entry in entries {
            let source = source.join(entry);
            let destination = destination.join(entry);
            if !source.exists() {
                continue;
            }

            debug!(
                "Migrating cache {kind} from `{}` to `{}`",
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
    use super::migrate_windows_cache;

    #[test]
    fn migrate_expected_windows_cache_entries() -> std::io::Result<()> {
        let source = tempfile::tempdir()?;
        let destination = source.path().join("cache");
        fs_err::create_dir(source.path().join("wheels-v1"))?;
        fs_err::write(source.path().join("wheels-v1").join("entry"), "wheel")?;
        fs_err::write(source.path().join("CACHEDIR.TAG"), "tag")?;
        fs_err::write(source.path().join("unrelated"), "untouched")?;

        migrate_windows_cache(source.path(), &destination)?;

        assert_eq!(
            fs_err::read_to_string(destination.join("wheels-v1").join("entry"))?,
            "wheel"
        );
        assert_eq!(
            fs_err::read_to_string(destination.join("CACHEDIR.TAG"))?,
            "tag"
        );
        assert_eq!(
            fs_err::read_to_string(source.path().join("unrelated"))?,
            "untouched"
        );
        Ok(())
    }
}
