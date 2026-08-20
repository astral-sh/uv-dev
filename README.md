# uv sync resolves relative paths in subdependencies different when run fresh or with uv.lock file

Issue: astral-sh/uv#21244

Classification: bug

## Summary

The reported fresh-versus-locked inconsistency is reproducible with a synthetic local Git fixture.
The fixture has a package in a Git subdirectory whose `[tool.uv.sources]` maps a transitive
dependency to a wheel elsewhere in the same repository. A first `uv sync` resolves all three
packages and writes a correct lockfile, but installation requests the wheel from an absolute path
under the downstream project and fails because that path does not exist. Repeating the same command
with the generated lockfile requests the repository-relative wheel path and succeeds.

The reproduction used uv 0.12.5 on x86_64 Linux with CPython 3.12.3. The report additionally lists
uv 0.12.0 and 0.12.5, macOS and Linux, and Python 3.12.12 and 3.13.11.

## Classification

This is a reproducible bug. The same dependency graph identifies different installation paths
depending only on whether resolution occurred in the current `uv sync` invocation or was replayed
from the lockfile. In both cases, the wheel is part of the enclosing Git repository and should be
addressed by the same repository-relative path.

Merged astral-sh/uv#10072 establishes that pre-built archives within Git repositories are supported.
The existing integration test verifies lockfile serialization and installation from an existing
lockfile, but does not exercise this fresh-sync and Git-subdirectory combination.

## Reproduction

Outcome: reproducible.

The reproduction was constructed entirely under `/tmp`, with a dedicated uv cache and temporary
directory. The local Git repository had this layout:

```text
upstream/
├── test-lib/
│   └── pyproject.toml
└── test-wheel/
    └── dist/test_wheel-0.1.0-py3-none-any.whl
```

The relevant metadata in `upstream/test-lib/pyproject.toml` was:

```toml
[project]
name = "test-lib"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["test-wheel"]

[tool.uv.sources]
test-wheel = { path = "../test-wheel/dist/test_wheel-0.1.0-py3-none-any.whl" }
```

The downstream project selected that package from a Git subdirectory:

```toml
[project]
name = "downstream"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["test-lib"]

[tool.uv.sources]
test-lib = { git = "file:///tmp/.../upstream", subdirectory = "test-lib" }
```

With uv 0.12.5 (`x86_64-unknown-linux-gnu`) and CPython 3.12.3, the first command was run with an
empty project state and dedicated cache:

```console
$ UV_CACHE_DIR=/tmp/.../cache TMPDIR=/tmp/.../tmp UV_PYTHON_DOWNLOADS=never \
    uv sync --python /usr/bin/python3 --verbose
```

It exited 1 after resolving three packages. The resolver logged the correct Git-relative source:

```text
test-wheel @ git+file:///tmp/.../upstream#path=test-wheel/dist/test_wheel-0.1.0-py3-none-any.whl
```

The installation request instead used the downstream project as the path base:

```text
test-wheel @ git+file:///tmp/.../upstream@18e650a...#path=/tmp/.../downstream/test-wheel/dist/test_wheel-0.1.0-py3-none-any.whl
```

It then failed with `failed to query metadata of file` and `No such file or directory`. Despite the
failure, `uv.lock` was written with the correct source:

```toml
source = { git = "file:///tmp/.../upstream?path=test-wheel%2Fdist%2Ftest_wheel-0.1.0-py3-none-any.whl#18e650a..." }
```

Running the identical command again exited 0. uv reported that the existing lockfile satisfied the
project requirements, requested `#path=test-wheel/dist/test_wheel-0.1.0-py3-none-any.whl`, and
installed `downstream`, `test-lib`, and `test-wheel`.

Existing coverage: `crates/uv/tests/sync/sync.rs`, test `sync_git_path_archive`, creates a Git-root
dependency that transitively references a wheel in the same repository. It runs `uv lock` first,
asserts the repository-relative wheel source in `uv.lock`, and then runs `uv sync` successfully from
that lockfile. It does not run `uv sync` without a pre-existing lockfile, and the parent Git package
is at the repository root rather than selected with `subdirectory`.

## Related

- astral-sh/uv#10072 (merged pull request), “Add support for direct archive dependencies in Git” —
  Added support for pre-built wheels and source archives inside Git repositories. Its
  `sync_git_path_archive` coverage verifies an archive dependency at the repository root via
  `uv lock` followed by locked `uv sync`.
- astral-sh/uv#9516 (closed issue), “Adding Git repo at subdirectory that points to source in same
  repo does not work” — Similar repository topology for a sibling directory dependency. It differs
  because the machine-local checkout path was written into the lockfile; astral-sh/uv#9594 fixed
  that case by expressing the dependency relative to the Git root.
- astral-sh/uv#19152 (closed issue), “uv lock with transitive poetry path depedencies results in
  machine-specific path in lockfile” — Another transitive relative-path source identity issue.
  astral-sh/uv#19269 preserved the enclosing Git source for directory dependencies. It differs from
  astral-sh/uv#21244 because the new report's lockfile is already correct and the bad absolute path
  appears only in the fresh sync's installation request.

## Maintainer notes

The observed mismatch is sufficient to reproduce the issue, but this handoff does not claim a
confirmed source-level root cause. A regression test needs both distinguishing conditions: the
parent dependency selected from a Git `subdirectory`, and `uv sync` performing resolution and
installation without a pre-existing lockfile.
