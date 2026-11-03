# link-mode = "symlink" breaks mlx library on macOS due to unresolved @rpath/libmlx.dylib

Issue: astral-sh/uv#23072

Classification: bug

## Summary

The reporter says importing `mlx.core` fails after installing MLX with `link-mode = "symlink"`. The reported environment is Apple Silicon, macOS 27.2, uv 0.13.6 (October 26, 2026), and uv-managed Python 3.13.16. They report that `clone` and `copy` work.

The proposed reproduction initializes a project, writes the symlink setting to `uv.toml`, runs `uv add mlx`, then runs `uv run python -c "import mlx.core"`. Installation succeeds; the import raises `ImportError: Library not loaded: @rpath/libmlx.dylib`. The loader identifies `mlx/core.cpython-313-darwin.so` in an `archive-v0` cache entry and searches that entry's `mlx/lib/libmlx.dylib`, followed by the managed Python installation's library directory.

No canonical duplicate or matching fix found. astral-sh/uv#8953 is an adjacent loader-path failure; astral-sh/uv#5244 and astral-sh/uv#5208 document symlink-mode design and implementation history.

The issue currently has the bug label and no comments. The report does not give the resolved MLX package versions. Its traceback includes `mlx_spatial/checkpoint.py`, which the minimal example does not explicitly install. The behavior has not been reproduced during this triage, and the reporter's proposed root cause is not independently confirmed.

## Draft response

The traceback shows macOS loading `mlx.core` from uv's cache and looking for `libmlx.dylib` within that cached wheel's directory. uv currently symlinks individual package files; the exact MLX library layout still needs confirmation.

As a workaround, set `link-mode = "clone"` (the macOS default) and run `uv sync --reinstall` to replace the installed symlinks.

Please share the exact MLX-related package versions and confirm whether the fresh-project example fails on its own. The traceback also includes `mlx_spatial`, which isn't shown in that example. Those details will help establish the affected wheel layout.

## Classification

The report describes an installed extension failing to load a required library specifically with supported symlink mode, while clone and copy work. Source confirms per-file symlinking, but the exact MLX wheel layout and loader mechanism remain unverified. The documented cache-deletion warning does not explain this intact-cache import failure. No inspected item tracks the same failure or establishes a previously fixed regression.

The source establishes the installation strategy, not the complete dynamic-loader mechanism. In particular, there is no inspected lockfile or wheel manifest showing which distribution contains `libmlx.dylib`, whether it shares an archive with the extension, or the extension's Mach-O run paths. A layout assembled from multiple wheel archives is a possibility to investigate, not an established diagnosis.

The earlier symlink feature and directory-linking discussions predate this report by more than two years. Neither is evidence that this particular failure was fixed and returned. The adjacent libpython issue concerns a different library and trigger, so it is not a canonical duplicate.

## Related

- astral-sh/uv#8953 — MuJoCo `mjpython` fails with uvx (issue, open). Adjacent macOS loader failure: a maintainer identified libpython being sought relative to a cached environment instead of the managed interpreter. It concerns @executable_path and libpython under uvx, whereas this report concerns @rpath/libmlx.dylib when package files are installed with symlink mode; it does not establish a shared root cause.
- astral-sh/uv#5244 — Symlink link mode should symlink at the directory level (issue, closed). Relevant installer design history: directory-level symlinking was proposed, then reconsidered by a maintainer because compiled .pyc files would pollute the cache. Current source still symlinks individual files. This discussion neither reports nor fixes native-library resolution across symlinked files.
- astral-sh/uv#5208 — Support  `--link-mode=symlink` (pull request, merged). Introduced symbolic linking for dependency installation and syncing on July 19, 2024; discovered through astral-sh/uv#5147. This establishes the feature's history, but provides no fix for MLX or @rpath resolution and no evidence that the reported failure is a regression.

## Supporting evidence

- `crates/uv-install-wheel/src/linker.rs:253`: `link_wheel_files` installs the unpacked wheel into site-packages using `link_dir`, merging destination directories. It requests a real copy of `RECORD` because installation modifies it.
- `crates/uv-fs/src/link.rs:74` and `crates/uv-fs/src/link.rs:356`: symlink mode walks the source tree, creates destination directories, and links individual files.
- `crates/uv-fs/src/link.rs:701`: `symlink_file_with_fallback` links a source file to its destination, with a copy fallback. This code does not itself establish how macOS resolves an MLX dependency.
- `crates/uv-fs/src/link.rs:39` and `crates/uv-settings/src/settings.rs:1180`: clone mode is the macOS default. The setting documentation discourages symlinks because deleting the cache removes their targets; it does not document this specific native-library failure.
- `crates/uv-cli/src/lib.rs:7083`: reinstall explicitly reinstalls already-installed packages, supporting the proposed workaround instead of merely changing the configuration.
- In astral-sh/uv#8953, maintainer zanieb explains that MuJoCo searches for libpython relative to the wrong executable base. The referenced astral-sh/uv#8879 concerns copied managed-Python executables and upstream interpreter behavior, rather than symlinked MLX package files.
- In astral-sh/uv#5244, maintainer charliermarsh reconsidered directory symlinking because it would populate the cache with compiled Python files. Its closure does not indicate an MLX fix.
- astral-sh/uv#5147 explicitly identifies astral-sh/uv#5208 as the implementation of symlink mode. The pull request merged July 19, 2024; its body and comments contain no MLX-specific fix.
- `CHANGELOG.md` lists uv 0.13.6 on October 26, 2026 and 0.13.7 on November 3, 2026. Neither entry identifies a fix for this failure. There is no evidence supporting an upgrade-as-fix recommendation.

## Search coverage and candidates ruled out

The report was decomposed before searching into: the native import/loader error; successful installation followed by failure under `uv run`; the difference between symlink and clone/copy modes; cache-relative lookup as a proposed mechanism; and the reported uv/Python/platform versions. Observable symptoms were searched separately from potential symlink-resolution causes.

Used authenticated gh to search open and closed issues and open, closed, and merged PRs. Literal searches covered libmlx.dylib, libmlx, mlx, @rpath, and Library not loaded. Separate conceptual searches covered symlink imports, symlink library/dylib failures, link-mode imports, realpath, __file__, dynamic linking, shared libraries, and namespace packages, removing MLX, version, and platform restrictions. Fix-oriented PR searches covered symlink, link-mode, symbolic, rpath, dylib, mlx, and shared library; followed astral-sh/uv#5147 to astral-sh/uv#5208 and inspected the 0.13.6/0.13.7 changelog. Some supplemental queries were rate-limited; subsequent narrower searches succeeded. Ruled out astral-sh/uv#19126 (archived interpreter relocation), astral-sh/uv#11554 (missing cloned .so files), astral-sh/uv#12869 (upstream CMake installation path), astral-sh/uv#8762 (MLX wheel selection), and astral-sh/uv#7764 (Homebrew library search paths). No matching fix or canonical duplicate was found.

Additional distinctions from inspected candidates:

- astral-sh/uv#19126 concerns Linux Python executables copied during ZIP transport, not an extension imported immediately after package installation. Its maintainer points to environment export and portability discussions.
- astral-sh/uv#11554 concerns coremltools extension files missing after cloning. The present report locates and loads the extension but cannot resolve its dependent dylib.
- astral-sh/uv#12869 was resolved by fixing the package's CMake installation path and also failed with copy mode.
- astral-sh/uv#8762 concerns deployment-target compatibility during MLX wheel selection; the present installation completes.
- astral-sh/uv#7764 concerns Homebrew libraries absent from managed Python's library search paths, with no demonstrated package link-mode trigger.
- astral-sh/uv#19185 concerns source-built extensions cached against a different host's system libraries.
- astral-sh/uv#5359 concerns validation of wheel script entries, and astral-sh/uv#5360 concerns combining symlinks with an ephemeral cache. Neither trigger appears in this report.

## Next steps

Obtain the exact MLX-related distribution versions and confirmation that the fresh-project example reproduces independently of the larger project. Then inspect which wheels own the extension and dylib, their installed symlink targets, and the extension's run paths on macOS. Use that evidence to determine the appropriate installer or upstream packaging change.

## Handoff scope

Only this temporary README was updated. No checkout files or GitHub state were changed. No builds or runtime reproduction were attempted.
