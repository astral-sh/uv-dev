# Interpreter discovery aborts on a PATH candidate that fails to execute (pyenv shim, exit 127) instead of skipping it

Issue: astral-sh/uv#21047

Classification: bug report, not reproduced

## Summary

The report says `uv run --no-project --python 3.13` aborts when the first Python-like executable on `PATH` is a pyenv-style `python` shim that exits 127, even though a working Python 3.13 is available later on `PATH`.

Testing with uv 0.12.3 confirms that uv surfaces the reported diagnostic when no later interpreter satisfies 3.13, but does not confirm that discovery aborts at the shim. With a real Python 3.13 later on `PATH`, uv logs that it is skipping the bad interpreter, selects the later interpreter, and runs successfully.

The report's standalone fixture creates only the broken `python`; it does not create or identify the claimed later Python 3.13. The failing output from that incomplete fixture therefore does not demonstrate the reported selection bug.

## Classification

Treat this as a plausible platform- or configuration-dependent bug report, but the described behavior is not reproducible from the supplied fixture on Linux. The result should be revisited with the reporter's complete macOS discovery trace and actual interpreter paths. A query process exiting 127 is non-critical in the tested implementation; the first such error is retained and reported only if discovery finds no usable interpreter.

## Reproduction

Outcome: **not reproducible**.

Environment:

- uv 0.12.3 (`x86_64-unknown-linux-gnu`)
- Linux 6.17.0-1020-azure, x86_64
- Python 3.13.15 used as the later matching PATH candidate
- All scripts, Python installations, caches, and install directories were isolated under `/tmp/uv-21047-repro`

The failing candidate was reconstructed as an executable named `python` in the first PATH directory:

```sh
#!/bin/sh
echo "simulated shim: python unavailable" >&2
exit 127
```

A real Python 3.13.15 executable was placed in a later PATH directory, while the managed-installation directory used by the test was empty. The targeted command was:

```sh
PATH=/tmp/uv-21047-repro/broken:/tmp/uv-21047-repro/downloaded/cpython-3.13.15-linux-x86_64-gnu/bin:/usr/bin:/bin \
UV_CACHE_DIR=/tmp/uv-21047-repro/exact-cache \
UV_PYTHON_INSTALL_DIR=/tmp/uv-21047-repro/empty-install \
UV_PYTHON_DOWNLOADS=never \
uv -vv run --no-project --python 3.13 /tmp/uv-21047-repro/probe.py
```

The trace showed:

```text
Searching PATH for executables: python3.13, python3, python
Found possible Python executable: /tmp/uv-21047-repro/broken/python
Skipping bad interpreter at /tmp/uv-21047-repro/broken/python ... exit status: 127
Checking `PATH` directory for interpreters: .../cpython-3.13.15-linux-x86_64-gnu/bin
Found `cpython-3.13.15-linux-x86_64-gnu` ... (search path)
Using Python 3.13.15 interpreter ...
```

The probe printed Python 3.13.15 and uv exited 0. A second control requested Python 3.12 with the same broken first directory and `/usr/bin/python3.12` later on PATH; it also skipped the shim, ran successfully with Python 3.12.3, and exited 0.

The report's failing result was reproduced only after removing the matching 3.13 directory from PATH. In that case uv still continued past the shim, inspected `/usr/bin/python3` and `/usr/bin/python`, rejected both because they were Python 3.12.3 rather than 3.13, and only then returned the retained exit-127 error with exit code 2. Thus the diagnostic alone is not evidence that scanning stopped early.

No integration test directly exercises `uv run` selecting a matching later PATH interpreter after an earlier candidate returns a nonzero status. Related coverage includes:

- `crates/uv/tests/pip_install/pip_install.rs`, `install_incompatible_python_version_interpreter_broken_in_path`: verifies that the first broken candidate's query error is reported when no interpreter is usable for the requested virtual-environment operation, and that this retained error does not replace the normal no-matching-environment result when the broken candidate comes later.
- `crates/uv/tests/pip_compile/pip_compile.rs`, `compile_fallback_interpreter_broken_in_path`: verifies that a broken PATH interpreter does not prevent the command from falling back to an available interpreter for dependency builds.
- `crates/uv/tests/python/python_list.rs`, `python_list_ignores_noncritical_explicit_path_errors`: verifies that non-critical query failures are ignored by exhaustive listing.

To investigate the macOS-specific report, request the complete failing `-vv` output, `type -a python3.13 python3 python`, and successful `--version` output for every claimed later Python 3.13 executable. Those details should show whether uv reaches the later candidate and, if so, why it rejects it. The relevant differences from this reproduction are macOS 15 arm64, the real pyenv shim environment, and the naming/location of the claimed later 3.13 interpreter.

## Related

- astral-sh/uv#13402 — closed pyenv report with the same exit-127 diagnostic. A maintainer explained that uv skips the query error during discovery and resurfaces it if no appropriate interpreter is found.
- astral-sh/uv#10716 — merged change that retains the first non-critical Python discovery error and surfaces it after no usable interpreter is found.
- astral-sh/uv#15667 — concerns a critical spawn failure (`Exec format error`) during exhaustive listing, unlike a shim process that runs and returns status 127.
- astral-sh/uv#15315 — fixed a related pyenv-win case in which a discovered shim could not be queried.
