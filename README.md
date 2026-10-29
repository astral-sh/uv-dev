# `uvx` "Failed to spawn" on CPython dev builds with `+dev` version suffix

Issue: astral-sh/uv#22989

Classification: bug

## Summary

Reproduced on Ubuntu 24.04.5 x86_64 with the installed uv 0.13.5 and an unmodified, locally built CPython 3.16.0a1+dev. Running `uvx --python ./python pycowsay hello` installs the tool, then exits with code 2 and the reported `Failed to spawn` / `No such file or directory (os error 2)` error.

The installed `pycowsay` script exists, but its absolute shebang points to the temporary environment location that no longer exists after uv moves the environment into its cache. The cached environment's Python works and runs the script successfully when invoked explicitly. A separate metadata-only control confirms that adding +dev to `version_info` causes uv to generate this non-relocatable launcher despite `relocatable = true`.

The reporter observed the failure with Python 3.15 and 3.16 development builds on macOS 15 arm64 with uv 0.13.6 and Ubuntu 24.04 x86_64 with uv 0.12.23; Python 3.14.8+ worked. The report also names tox. The reproduction used pycowsay 0.0.0.2 and the installed uv version available on this runner, rather than those exact reported uv versions.

## Classification

Confirmed bug in environment metadata handling and tool launcher relocation. The +dev suffix is intentional upstream, as documented in python/cpython#99968. Interpreter discovery accepts it and environment creation writes it to `pyvenv.cfg`, but the configuration reader rejects local versions. The installer then treats the environment as non-relocatable and writes an absolute interpreter path. Moving the environment leaves that path dangling.

This differs from the historical bare-trailing-plus parsing issue astral-sh/uv#1357, fixed by astral-sh/uv#1771. There is no evidence that the older fix regressed. The observed failure concerns the newly exposed +dev metadata path; astral-sh/uv#13992 and astral-sh/uv#20849 concern adjacent spawn diagnostics. No matching duplicate was found in the related-issue investigation.

## Reproduction

**Outcome: reproducible.** Expected behavior: print a cow saying `hello` and exit successfully. Actual behavior: tool installation succeeds, then spawning the console entry point fails with ENOENT.

### Environment and setup

- Platform: Ubuntu 24.04.5 LTS, Linux x86_64.
- Installed executable: `/opt/hostedtoolcache/uv/0.13.5/x86_64/uvx`; `uv --version` reports `uv 0.13.5 (x86_64-unknown-linux-gnu)`.
- CPython: upstream commit `386d9d071cee2168bee135bc15897a27377d0999`, downloaded from the official python/cpython source archive and extracted under `/tmp/uv-22989-s_w7v_rd/cpython`. The source was not patched.
- Build: `./configure --prefix=/tmp/uv-22989-s_w7v_rd/install --without-ensurepip`, followed by `make -j4`; both exited 0. Compiler: GCC 13.3.0. The interpreter was run directly from the build tree; no system installation was performed. Eight optional extension modules were unavailable, with no module import failures in the build check.
- Interpreter output: `Python 3.16.0a1+dev`; `platform.python_version()` also returns `3.16.0a1+dev`.
- Tool: pycowsay 0.0.0.2 from PyPI. No project, workspace, dependency groups, or custom package configuration is needed.
- All reproduction files, environments, tool directories, temporary files, and caches were placed under `/tmp/uv-22989-s_w7v_rd`. Subprocesses received a clean explicit environment with `UV_NO_CONFIG=1`, `UV_PYTHON_DOWNLOADS=never`, and temporary `UV_CACHE_DIR`, `UV_TOOL_DIR`, `UV_TOOL_BIN_DIR`, `UV_PYTHON_INSTALL_DIR`, `TMPDIR`, and XDG directories. No inherited credential variables were passed.

### Commands and observed output

With that isolated environment, from the CPython build directory:

```console
$ ./python --version
Python 3.16.0a1+dev
$ uvx --python ./python pycowsay hello
Installed 1 package in 2ms
error: Failed to spawn: pycowsay
  cause: No such file or directory (os error 2)
```

The uvx exit code was 2; stdout was empty. The Linux build executable is named `python`, corresponding to the reporter's `python.exe` in their macOS build tree.

The resulting environment was `/tmp/uv-22989-s_w7v_rd/cache/uv/archive-v0/p2F159iSWyyBOLkE`. Its configuration contains:

```ini
home = /tmp/uv-22989-s_w7v_rd/cpython
implementation = CPython
uv = 0.13.5
version_info = 3.16.0a1+dev
include-system-site-packages = false
relocatable = true
```

Its existing `bin/pycowsay` begins with:

```text
#!/tmp/uv-22989-s_w7v_rd/cache/uv/builds-v0/.tmpnxTB9A/bin/python
```

That interpreter target does not exist. The archived environment's `bin/python` does exist. Executing the script with that interpreter succeeds:

```sh
/tmp/uv-22989-s_w7v_rd/cache/uv/archive-v0/p2F159iSWyyBOLkE/bin/python \
  /tmp/uv-22989-s_w7v_rd/cache/uv/archive-v0/p2F159iSWyyBOLkE/bin/pycowsay hello
```

This exits 0 and prints the expected cow saying `hello`, establishing that the missing shebang interpreter causes the reproduced spawn failure. The package emits a Python escape-sequence SyntaxWarning when run directly, which does not prevent execution.

### Controls confirming the metadata mechanism

The same isolated `uvx --python /usr/bin/python3 pycowsay hello` succeeds with system CPython 3.12.3 (exit 0).

A separate relocatable environment using that same system interpreter isolates the metadata effect:

```sh
uv venv --python /usr/bin/python3 --relocatable metadata-control
uv pip install --python metadata-control/bin/python pycowsay==0.0.0.2
```

Initially, `version_info = 3.12.3` produces a relative shell launcher starting with `#!/bin/sh` and executing `python` relative to the script's directory. Changing only this temporary environment's configuration to `version_info = 3.12.3+dev`, while retaining `relocatable = true`, and running:

```sh
uv pip install --python metadata-control/bin/python --reinstall pycowsay==0.0.0.2
```

produces an absolute shebang pointing at `metadata-control/bin/python`. Renaming the environment to `metadata-control-relocated` makes executing its existing `bin/pycowsay` fail with `FileNotFoundError`, errno 2. Restoring the original version metadata, reinstalling pycowsay, and renaming the environment again produces a launcher that executes successfully (exit 0). This control uses deliberately edited metadata; the primary 3.16 reproduction above uses an unmodified interpreter and uv-generated configuration.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/`, including the current tool, Python, and pip integration-test modules. No test covering a +dev interpreter version in virtual-environment metadata was found. Relevant adjacent tests were read, including their setup and assertions:

- `crates/uv/tests/tool/tool_run.rs::tool_run_cache` uses ordinary Python 3.11/3.12, runs black successfully, checks cached reuse and refresh behavior, and verifies that changing the interpreter or dependencies creates another environment. It does not exercise +dev metadata.
- `crates/uv/tests/pip_install/pip_install.rs::install_relocatable` creates a relocatable Python 3.12 environment, installs a local console-script fixture, and asserts execution before and after moving the environment and through a symlink on Unix. It does not include a local-version suffix.
- `crates/uv/tests/python/venv.rs::verify_pyvenv_cfg_relocatable` uses ordinary Python 3.12 and checks `relocatable = true` and activation-script contents. It does not test the +dev configuration round trip or a relocated installed console script.

The tool and pip-install modules are gated by `test-python` and `test-pypi`; the venv module is gated by `test-python`. These Rust tests were inspected, not executed. No repository test files were added or modified.

### Scope and retained evidence

The primary observation covers CPython 3.16.0a1+dev on Linux with uv 0.13.5. macOS, CPython 3.15, the reporter's Python 3.14.8+ control, tox, and uv 0.12.23/0.13.6 were not separately run. Those limits do not prevent reproduction of the reported command and error with a genuine development interpreter.

Build logs, the exact CPython commit, the isolated environment settings, baseline and failing command outputs, launcher/configuration evidence, and metadata-control results are retained under `/tmp/uv-22989-s_w7v_rd`. In particular, see `dev.json`, `dev.stderr`, `dev-environment.json`, `cpython-version.txt`, and `metadata-control-results.json`.

## Related

- astral-sh/uv#1357 — Support Python versions with trailing `+` (issue, closed). Historical CPython development-build compatibility issue: versions such as 3.11.4+ failed during interpreter version parsing. Maintainer discussion led to stripping the bare trailing plus. The new report reaches tool spawning with a valid +dev local version; the source exposes a different pyvenv.cfg parsing and relocation path.
- astral-sh/uv#1771 — strip trailing `+` from version number of local Python builds (pull request, merged). Merged on 2024-02-20 to fix astral-sh/uv#1357 by stripping a final + from python_full_version. That workaround remains in the checkout and deliberately leaves +dev intact. It does not fix rejection of local versions when reading pyvenv.cfg, so this report does not establish a regression of that fix.
- python/cpython#99968 — Python version ending with plus sign not compatible with PEP 508 `python_full_version` marker (issue, closed). Verified upstream context for the trigger. CPython maintainers confirmed on 2026-05-27 that upcoming 3.15+ development builds would use +dev; a June comment shows 3.15.0b2+dev. This establishes that the suffix is intentional, but does not track uv's tool-launch failure.
- astral-sh/uv#13992 — Improve error message on "failed to spawn" where the Python interpreter is missing (issue, open). Tracks the same misleading spawn/ENOENT symptom when an entry point exists but its shebang interpreter is missing. This is relevant to the stale interpreter path exposed by the source analysis, but requests better diagnostics rather than correcting +dev metadata parsing and tool-environment relocation.
- astral-sh/uv#20849 — Diagnose dangling shebang interpreters in `uv run` / `uv tool run` (pull request, open). Proposes diagnostics for astral-sh/uv#13992, including uv tool run. It explains how an existing script with a missing shebang interpreter yields ENOENT. The proposed change only adds a hint after spawning fails; it does not repair the +dev configuration parsing defect.

## Supporting source evidence

Inspected checkout: `d2152bc80fe9b9ef3fe6acceffd408d9bdd90b23`.

1. `crates/uv-python-interpreter/python/get_interpreter_info.py:65` obtains `python_full_version` from `platform.python_version()`; lines 98–99 strip only a bare final plus. A +dev suffix survives.
2. `crates/uv-virtualenv/src/virtualenv.rs:556` writes the full version into `version_info` for environments not using a managed minor-version link. The same function writes `relocatable = true` at line 573 when requested.
3. `crates/uv-python-interpreter/src/virtualenv.rs:143` parses `version` and `version_info` with `PythonVersion::from_str`. In `crates/uv-python-types/src/python_version.rs:49`, that parser rejects any local version. Under PEP 440, +dev is a local-version label, not a .dev prerelease segment.
4. `crates/uv-python-interpreter/src/environment.rs:287` implements relocatability as `self.cfg().is_ok_and(|cfg| cfg.is_relocatable())`. A configuration parse failure therefore returns false even if the file contains `relocatable = true`.
5. `crates/uv-installer/src/installer.rs:122` passes that result to wheel installation. `crates/uv-install-wheel/src/wheel.rs:979` keeps the absolute executable path when relocatability is false; line 128 formats the shebang.
6. `crates/uv-environment-operations/src/environment.rs:247` creates a relocatable temporary tool environment, installs into it, and subsequently persists it at the cache archive location. Absolute entry-point interpreter paths can consequently refer to the old temporary location.
7. `crates/uv-tool-commands/src/run.rs:432` prepends the environment's scripts directory to PATH; line 451 attempts to spawn the tool, and line 467 adds the reported Failed to spawn context.

The runtime reproduction confirms the stale absolute interpreter path after cache relocation. The metadata-only control also demonstrates that adding +dev to version_info defeats relocatable entry-point generation. The generic ENOENT message alone would not distinguish a missing command from a missing shebang interpreter; the generated script and successful explicit-interpreter invocation establish that distinction here.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues for the exact spawn error, python.exe, +dev, 3.15/3.16, 3.16.0a1, and 99968; expanded to development/self-built CPython, prereleases, local versions, sysconfig, pyvenv.cfg/version_info, relocatable environments, and shebangs. Kept symptom searches separate from source-derived causes. PR search/list queries returned empty results even for a known merged fix, so additionally searched titles and bodies through the complete paginated REST pull-request listing with state=all, and inspected linked PRs, comments, reviews, and timelines. Some supplemental search queries were rate-limited. Ruled out astral-sh/uv#15967 and astral-sh/uv#15987, fixed by astral-sh/uv#15990 in 0.8.20, because they require disabled caching; astral-sh/uv#13233 and merged astral-sh/uv#13234 concern prerelease environment invalidation under uv run --no-sync; astral-sh/uv#15692 concerns system-site-package visibility, not interpreter suffixes. No existing tracker for the +dev metadata/relocation defect was found.

Further inspected candidates were also distinct:

- astral-sh/uv#12000 reused an environment after a Docker Python-version change and raised ModuleNotFoundError; the reporter confirmed resolution after uv 0.7.14.
- astral-sh/uv#19920 concerned managed minor-version links and pre-commit's version health check. Maintainer comments identify shortened major/minor metadata, not local-version rejection.
- astral-sh/uv#11796 concerns invoking an unavailable versioned Python command; its comments point to diagnostic work in astral-sh/uv#12201.
- astral-sh/uv#7604 concerns pyenv shell configuration and interpreter selection. astral-sh/uv#16231 concerns copying a virtual environment between different Python patch versions. Neither establishes the +dev failure.

The related-issue investigation used the authenticated GitHub CLI only for reads. Searches included historical closed issues and merged fixes as well as open work. The upstream lead was verified against its discussion rather than assumed to establish the uv failure's cause.

## Suggested follow-up

Ensure Python versions emitted into environment metadata can be read back, and that a metadata parse failure cannot silently disable the relocatability needed by cached tool environments. Regression coverage should exercise +dev metadata and a console entry point after relocation, following the existing integration-test style and feature gates above.

The reproduction required only a temporary CPython build and the installed uv executable. No uv build was performed, no repository files were changed, and no GitHub objects were modified. Only this issue-context README and temporary reproduction artifacts were written.
