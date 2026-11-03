# link-mode = "symlink" breaks mlx library on macOS due to unresolved @rpath/libmlx.dylib

Issue: astral-sh/uv#23072

Classification: bug (reported; macOS runtime failure not independently reproduced)

## Summary

The reporter says `uv run python -c "import mlx.core"` raises `ImportError: Library not loaded: @rpath/libmlx.dylib` after installing MLX with `link-mode = "symlink"`. The reported environment is Apple Silicon, macOS 27.2, uv 0.13.6 (329bf9448, October 26, 2026), and uv-managed Python 3.13.16. They report successful imports with `clone` and `copy`; no upgrade regression or last known-good uv version is stated.

The proposed fresh-project example initializes a project, writes `link-mode = "symlink"` to `uv.toml`, adds `mlx`, and imports `mlx.core`. The actual traceback comes from a larger project, includes `mlx_spatial/checkpoint.py`, and identifies the extension in an `archive-v0` cache entry. The resolved MLX-related distribution versions are omitted.

A Linux investigation installed actual Apple Silicon wheels for CPython 3.13 and confirmed that the extension and dylib belong to different distributions and resolve into different cache archives. It also inspected the extension's Mach-O dependencies and run path. Those observations support the proposed explanation, but macOS dynamic-loader behavior was not executed. A native Linux CPU variant imported successfully with symlink mode.

## Classification

Retain the reported bug classification pending a macOS runtime reproduction. The report concerns an intact cache and an installed native extension failing to find a dependency, rather than the documented consequence of deleting symlink targets. The investigation confirms the relevant wheel layout for MLX 0.32.4, not the reporter's unspecified versions or the complete macOS loader mechanism. It does not establish whether an installer change or upstream packaging change is appropriate.

No canonical duplicate or matching fix was found in the earlier issue-context search. astral-sh/uv#8953 is an adjacent loader-path failure; astral-sh/uv#5244 and astral-sh/uv#5208 document symlink-mode design and implementation history. The inspected 0.13.6 and 0.13.7 changelog entries identify no matching fix, so an upgrade is not a verified remedy.

## Reproduction

**Outcome: `needs_more_information`.** The reported macOS import failure remains unverified. This runner is Linux x86-64 and cannot run Apple's dynamic loader or the Apple Silicon Mach-O extension. Successful Linux imports and cross-platform file installation do not disprove the report.

### Environment and isolation

- Installed executable: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, reporting `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Host: Linux x86-64, kernel `7.0.0-1017-azure`; system Python 3.12.3.
- Installed uv-managed CPython 3.13.16 into the temporary fixture for the native runtime check.
- All reproduction projects, virtual environments, caches, managed Python files, configuration directories, logs, and installation targets are under `/tmp/uv-23072-t6_jorbr`.
- Commands used a minimal subprocess environment with temporary `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `UV_PYTHON_BIN_DIR`, XDG directories, and `TMPDIR`. No inherited uv settings, activated environment, or configured private package index was used. Package installs explicitly used public PyPI. Cache entries remained intact throughout.
- MLX 0.32.4 was the current public PyPI release inspected. It is a selected fixture version, not a version supplied by the reporter.

### Apple Silicon installation and wheel inspection

The following commands both exited 0. The host Python performs installation only; `--python-version` and `--python-platform` select CPython 3.13 Apple Silicon wheels. `MACOSX_DEPLOYMENT_TARGET=27.2` matches the reported platform and selects macOS 26.0 arm64 wheels.

```sh
repro_root=/tmp/uv-23072-t6_jorbr
MACOSX_DEPLOYMENT_TARGET=27.2 uv --no-config pip install \
  --python /usr/bin/python3 --python-version 3.13.16 \
  --python-platform aarch64-apple-darwin --target "$repro_root/mac-symlink" \
  --link-mode symlink --only-binary :all: \
  --index-url https://pypi.org/simple 'mlx==0.32.4'

MACOSX_DEPLOYMENT_TARGET=27.2 uv --no-config pip install \
  --python /usr/bin/python3 --python-version 3.13.16 \
  --python-platform aarch64-apple-darwin --target "$repro_root/mac-copy" \
  --link-mode copy --only-binary :all: \
  --index-url https://pypi.org/simple 'mlx==0.32.4'
```

Observed distribution ownership, symlinks, and Mach-O load commands:

- `mlx==0.32.4` owns `mlx/core.cpython-313-darwin.so`.
- Its Darwin dependency `mlx-metal==0.32.4` owns `mlx/lib/libmlx.dylib`.
- In the symlink installation, these files exist but point to different `cache/archive-v0/<entry>/mlx/...` directories. `mlx/lib/libmlx.dylib` does not exist beside the resolved cached extension. This is a missing cache-relative candidate, not a broken installed symlink.
- Parsing the extension's Mach-O load commands found `LC_LOAD_DYLIB: @rpath/libmlx.dylib` and `LC_RPATH: @loader_path/lib`.
- In the copy installation, neither file is a symlink and the dylib exists at `lib/libmlx.dylib` relative to the resolved extension's directory. This confirms the layout difference; it is not a macOS import-success result.
- Neither installation contains `mlx/__init__.py` or `mlx_spatial/checkpoint.py`. The traceback's additional package context is not represented by this fresh fixture.

### Native Linux runtime check

Executed explicitly with the temporary cache and managed-Python directories:

```sh
cd "$repro_root"
uv --no-config python install 3.13.16 --no-bin
uv init --python 3.13.16 --vcs none --no-workspace native-project
cd native-project
printf 'link-mode = "symlink"\n' > uv.toml
uv add 'mlx[cpu]==0.32.4' --no-build --index-url https://pypi.org/simple
uv run python -c 'import mlx.core; print("mlx.core import succeeded")'
```

Every command exited 0; the import printed `mlx.core import succeeded`. A subsequent `uv run --no-sync` check confirmed Python 3.13.16, `mlx==0.32.4`, and `mlx-cpu==0.32.4`. The installed Linux extension and `libmlx.so` were symlinks to separate cache archives, and the library was absent beside the cached extension, yet the import succeeded. The CPU extra supplies the Linux backend; this is a platform-specific comparison, not the reported macOS command.

### Existing test coverage

Searched `crates/uv/tests/it/`, `crates/uv-client/tests/it/`, and the relevant relocated pip and sync integration modules for MLX, `@rpath`, and symlink-mode coverage. No MLX or macOS native-library-resolution test was found.

Read the setup and assertions of these nearby tests in `crates/uv/tests/pip/pip_sync.rs`:

- `install_symlink` (Unix): installs `MarkupSafe==2.1.3` under Python 3.12 with `pip sync --link-mode symlink --strict`, verifies importing `Markup` succeeds, deletes the fixture cache, then verifies the import fails. It does not test two distributions sharing an extension/library directory or macOS run paths.
- `install_symlink_no_cache`: installs the same requirement with `--link-mode symlink --no-cache --strict` and snapshots exit code 2 with the explicit unsupported-combination error. The reported command does not use `--no-cache`.

These tests do not establish coverage of the reported failure. No repository tests or builds were run or changed.

### Information and execution still needed

Run the fresh-project example on Apple Silicon macOS with uv 0.13.6 and Python 3.13.16, recording the exact `mlx`, `mlx-metal`, and any `mlx-spatial` versions (preferably the relevant `uv.lock` entries). Confirm whether adding only `mlx` fails independently of the larger project. Capture the installed extension and dylib symlink targets and `otool -L` / `otool -l` output for the extension. Then repeat the same pinned fixture after `uv sync --reinstall --link-mode copy` or `clone` and compare imports. Those results are needed to confirm the runtime failure and the reported workaround.

Investigation artifacts: `/tmp/uv-23072-t6_jorbr`, including `mac-layout.txt`, installation logs, `native-import-symlink.log`, and `native-verification.log`.

## Related

- astral-sh/uv#8953 — MuJoCo `mjpython` fails with uvx (open issue). Adjacent macOS loader failure involving libpython and `@executable_path`; it does not establish a shared cause with MLX's `@rpath` dependency. Its referenced astral-sh/uv#8879 concerns copied managed-Python executables and upstream interpreter behavior.
- astral-sh/uv#5244 — Symlink link mode should symlink at the directory level (closed issue). Directory-level symlinking was reconsidered because compiled `.pyc` files would pollute the cache. Its closure does not establish an MLX fix.
- astral-sh/uv#5208 — Support `--link-mode=symlink` (merged pull request, July 19, 2024), identified through astral-sh/uv#5147. Establishes feature history, with no MLX-specific fix or evidence of an upgrade regression.

The earlier search also distinguished astral-sh/uv#19126 (archived interpreter relocation), astral-sh/uv#11554 (missing cloned coremltools extensions), astral-sh/uv#12869 (upstream CMake installation paths, also failing with copy mode), astral-sh/uv#8762 (MLX wheel deployment-target selection), astral-sh/uv#7764 (Homebrew library lookup), astral-sh/uv#19185 (cached source builds tied to another host), astral-sh/uv#5359 (wheel script validation), and astral-sh/uv#5360 (symlinks with an ephemeral cache). None establishes a duplicate or fix for this report. That search covered open and closed issues and pull requests; some supplemental queries were rate-limited. No new GitHub search or write was performed during reproduction.

## Implementation context

`crates/uv-install-wheel/src/linker.rs::link_wheel_files` merges each wheel into site-packages through `link_dir` and copies mutable `RECORD` files. `crates/uv-fs/src/link.rs::walk_and_link` creates destination directories and links individual files; `symlink_file_with_fallback` implements file symlinking. The observed MLX installation agrees with this strategy. These source paths alone do not establish macOS runtime behavior.

`crates/uv-settings/src/settings.rs` documents clone as the macOS default and discourages symlinks because deleting the cache breaks their targets. Cache deletion was not part of either the reported trigger or this investigation.

## Suggested response

The wheel layout supports your explanation: MLX 0.32.4 puts the extension and `libmlx.dylib` in separate distributions, and symlink mode leaves the extension's cache-relative library location absent. We could verify that layout, but this runner is Linux and could not execute the macOS import. Please provide the exact MLX-related versions and confirm whether the fresh project with only `mlx` fails. The supplied traceback also includes `mlx_spatial`, which is absent from that example.

For the reported workaround, set `link-mode = "clone"` (the macOS default) and run `uv sync --reinstall` to replace installed symlinks. Import success with that workaround is reported by the author and still requires independent macOS verification.

## Handoff scope

Only this issue-context README was updated outside the checkout. All reproduction files and caches were created in the temporary fixture. No checkout files or GitHub state were changed by this investigation; no credentials were inspected or exposed.
