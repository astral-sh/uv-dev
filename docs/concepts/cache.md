# Caching

## Dependency caching

uv caches dependencies to avoid downloading or building them again.

The caching behavior depends on the dependency type:

- **For registry dependencies**, such as packages from PyPI, uv respects HTTP caching headers.
- **For direct URL dependencies**, uv respects HTTP caching headers and uses the URL as a cache key.
- **For Git dependencies**, uv uses the resolved Git commit hash. For example, `uv pip compile` pins
  each Git dependency to a specific commit hash.
- **For local dependencies**, uv uses the last-modified time of the local `.whl` or `.tar.gz` file.
  For directories, uv uses the last-modified time of `pyproject.toml`, `setup.py`, or `setup.cfg`.
- **For flat indexes**, such as `--find-links` locations, uv assumes that index contents do not
  change. It caches each file by name. If a file changes but keeps the same name, refresh the cache
  before uv can detect the new contents.

The following options resolve common caching issues:

- Run `uv cache clean` to clear the entire cache. Run `uv cache clean <package-name>` to clear the
  cache for one package. For example, `uv cache clean ruff` clears the cache for `ruff`.
- Pass `--refresh` to any command to revalidate cached data for all dependencies. For example, use
  `uv sync --refresh` or `uv pip install --refresh ...`.
- Pass `--refresh-package` to revalidate cached data for one dependency. For example, use
  `uv sync --refresh-package ruff` or `uv pip install --refresh-package ruff ...`.
- Pass `--reinstall` to an installation command to ignore installed versions. For example, use
  `uv sync --reinstall` or `uv pip install --reinstall ...`. Consider running
  `uv cache clean <package-name>` first to clear the package cache before reinstalling it.

uv always rebuilds and reinstalls local directory dependencies passed directly on the command line,
such as `uv pip install .`.

## Dynamic metadata

By default, uv rebuilds and reinstalls a local directory dependency only if its project metadata
changes. It checks `pyproject.toml`, `setup.py`, and `setup.cfg` in the directory root. It also
checks whether a `src` directory appears or disappears. This heuristic might not detect every change
that requires a reinstall.

To include more information in a package cache key, set
[`tool.uv.cache-keys`](../reference/settings.md#cache-keys). This setting supports file paths and
Git commit hashes. It replaces the default cache keys. Include every required default file, such as
`pyproject.toml`, in the custom keys.

For example, a project might declare dependencies in `pyproject.toml` and manage its version with
[`setuptools-scm`](https://pypi.org/project/setuptools-scm/). To rebuild when the dependencies or
Git commit change, add these cache keys to `pyproject.toml`:

```toml title="pyproject.toml"
[tool.uv]
cache-keys = [{ file = "pyproject.toml" }, { git = { commit = true } }]
```

If dynamic metadata depends on Git tags, include the tags in the cache key:

```toml title="pyproject.toml"
[tool.uv]
cache-keys = [{ file = "pyproject.toml" }, { git = { commit = true, tags = true } }]
```

If a project reads dependencies from `requirements.txt`, include that file in its cache keys:

```toml title="pyproject.toml"
[tool.uv]
cache-keys = [{ file = "pyproject.toml" }, { file = "requirements.txt" }]
```

The `file` key supports the glob syntax of the
[`glob`](https://docs.rs/glob/0.3.1/glob/struct.Pattern.html) crate. For example, use this pattern
to invalidate the cache when any `.toml` file in the project or its subdirectories changes:

```toml title="pyproject.toml"
[tool.uv]
cache-keys = [{ file = "**/*.toml" }]
```

!!! note

    Glob patterns can be expensive. uv might need to search large or deeply nested directories to
    detect changed files.

If a project depends on an environment variable, include that variable in its cache keys:

```toml title="pyproject.toml"
[tool.uv]
cache-keys = [{ file = "pyproject.toml" }, { env = "MY_ENV_VAR" }]
```

To invalidate the cache when a specific directory appears or disappears, include that directory:

```toml title="pyproject.toml"
[tool.uv]
cache-keys = [{ file = "pyproject.toml" }, { dir = "src" }]
```

The `dir` key tracks the directory itself. It does not track changes to files in that directory.

If `tool.uv.cache-keys` cannot capture a project's `dynamic` metadata, add the project to
`tool.uv.reinstall-package`. uv then rebuilds and reinstalls the project on every run:

```toml title="pyproject.toml"
[tool.uv]
reinstall-package = ["my-package"]
```

uv rebuilds and reinstalls `my-package` even when `pyproject.toml`, `setup.py`, and `setup.cfg` have
not changed.

## Cache safety

Multiple uv commands can run concurrently, even with the same virtual environment. The uv cache is
thread-safe and append-only, so multiple processes can read and write to it. During installation, uv
locks the target virtual environment to prevent concurrent changes.

_Never_ modify the cache directly, such as by removing a file or directory.

## Clearing the cache

Use these commands to remove cache entries:

- `uv cache clean` removes _all_ entries from the cache directory.
- `uv cache clean ruff` removes all cache entries for the `ruff` package.
- `uv cache prune` removes _unused_ cache entries and all centralized project environments. For
  example, it removes entries from older uv versions that are no longer necessary. uv recreates
  centralized project environments when needed. Run this command periodically to clean the cache.

By default, cache cleanup estimates the disk space reclaimed. Enable the `cache-physical-space`
[preview feature](./preview.md) for a more accurate estimate. This estimate accounts for hardlinks
and copy-on-write clones:

```console
$ uv cache clean --preview-features cache-physical-space
```

If uv cannot measure an entry's allocated size, it reports a lower bound from the remaining entries.
For example, this can occur with a compressed extent on Btrfs. The preview feature supports macOS
and Linux. Other platforms continue to report a coarser estimate of the space reclaimed.

uv blocks cache changes while other uv commands run. By default, `uv cache` commands wait up to 5
minutes for other uv processes to finish. This timeout prevents deadlocks. Set
[`UV_LOCK_TIMEOUT`](../reference/environment.md#uv_lock_timeout) to change the timeout. Use
`--force` to ignore the lock only when no other uv process is reading or writing to the cache.

## Caching in continuous integration

Continuous integration systems, such as GitHub Actions and GitLab CI, often cache package artifacts
to speed up later runs.

By default, uv caches both wheels that it builds from source and pre-built wheels that it downloads.

In continuous integration, caching pre-built wheels can be slower than downloading them again.
However, caching wheels built from source can save time because builds are often expensive. This is
especially true for packages with extension modules.

`uv cache prune --ci` removes pre-built wheels and unzipped source distributions from the cache. It
keeps wheels built from source. Run this command at the end of a continuous integration job to
reduce the cache size. For an example, see the
[GitHub integration guide](../guides/integration/github.md#caching).

## Cache directory

uv selects the first applicable cache directory:

1. A temporary cache directory if the command includes `--no-cache`.
2. The cache directory set by `--cache-dir`, `UV_CACHE_DIR`, or
   [`tool.uv.cache-dir`](../reference/settings.md#cache-dir).
3. A system-appropriate cache directory, e.g., `$XDG_CACHE_HOME/uv` or `$HOME/.cache/uv` on Unix and
   `%LOCALAPPDATA%\uv\cache` on Windows

!!! note

    uv _always_ requires a cache directory. With `--no-cache`, it uses a temporary cache to share
    data during the current command.

    In most cases, use `--refresh` instead of `--no-cache`. It updates the cache for later commands
    without reading existing cache entries.

Place the cache directory on the same file system as the target Python environment. Otherwise, uv
cannot link cached files into the environment and must copy them instead. Copying is slower.

## Cache versioning

The uv cache contains separate buckets for wheels, source distributions, Git repositories, and other
data. Each bucket has a version. If a release changes a cache format, uv does not read or write
incompatible buckets.

For example, uv 0.4.13 changed the core metadata bucket format and increased its version from v12 to
v13. Changes within the same cache version remain forwards- and backwards-compatible.

Because cache format changes also change the cache version, multiple uv versions can share a cache
directory safely. However, releases with different cache versions might not share the same cache
entries.

For example, uv 0.4.12 and uv 0.4.13 can share a cache directory. The core metadata bucket might
contain duplicate entries because its version changed.
