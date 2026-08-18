# uv pip install --python <venv> fails for a cpython-*-emscripten-wasm32-musl venv on Windows — malformed path mixing POSIX and Windows conventions

Issue: astral-sh/uv#21199

Classification: bug

## Summary

With uv 0.12.5 on Windows, explicitly selecting a uv-created Pyodide environment for
`uv pip install` loses the environment prefix. Both `--python <venv>` and
`--python <venv>\Scripts\python.exe` report the environment as `/` and derive an install destination
under `//lib/python3.13/site-packages` instead of the environment's `Lib\site-packages` directory.

The reported path-selection behavior was reproduced with the installed uv 0.12.5 by exercising
the Pyodide launcher's Windows-only branch in a controlled Linux run. A native Windows runner was
not available, so the final filesystem error differs: Linux reports read-only filesystem error 30,
whereas the report observes Windows network-path error 53 for the same root-level destination.

## Classification

This is a bug. An explicitly selected virtual environment must remain the installation root. In the
reproduction, both explicit selector forms instead changed the reported root to `/` and attempted
installation under `//lib/python3.13/site-packages`.

The controlled run also isolates the relevant platform condition. The Pyodide launcher patches
`sys.prefix` on Windows from `VIRTUAL_ENV`, defaulting to `/` when that variable is absent. Explicit
`--python` selection does not activate the environment or populate `VIRTUAL_ENV`; exercising that
branch reproduced the report. This explains why the activated-environment workflow added by
astral-sh/uv#17658 does not cover the failure.

## Reproduction

Outcome: reproducible.

Environment used:

- Installed `uv 0.12.5 (x86_64-unknown-linux-gnu)` on Linux x86_64.
- Pyodide `cpython-3.13.2-emscripten-wasm32-musl`.
- Fresh `UV_CACHE_DIR` and `UV_PYTHON_INSTALL_DIR` beneath `$RUNNER_TEMP` and `UV_NO_CONFIG=1`.
- Node's read-only-but-configurable `process.platform` property was overridden to `win32` from a
  temporary `NODE_OPTIONS=--require=...` shim. This exercises the downloaded Pyodide launcher's
  Windows branch without modifying the checkout or the downloaded runtime.

As a baseline, the unmodified Linux branch succeeded:

```console
$ uv venv venv-dir --python cpython-3.13.2-emscripten-wasm32-musl
$ uv pip install --python venv-dir --no-build click
Using Python 3.13.2 environment at: venv-dir
Installed 1 package in 1ms
 + click==8.4.2
$ uv pip install --python venv-exe/bin/python --no-build click
Using Python 3.13.2 environment at: venv-exe
Installed 1 package in 2ms
 + click==8.4.2
```

The temporary Windows-platform shim contained only:

```javascript
Object.defineProperty(process, "platform", { value: "win32" });
```

After creating a fresh Pyodide venv normally, the two targeted commands were run with that shim in
`NODE_OPTIONS` and with `VIRTUAL_ENV` unset:

```console
$ uv pip install --python simulated-venv --no-build click
Using Python 3.13.2 environment at: /
Resolved 1 package in 59ms
Prepared 1 package in 7ms
error: Failed to install: click-8.4.2-py3-none-any.whl (click==8.4.2)
  Caused by: Failed to create directory `//lib/python3.13/site-packages/`
  Caused by: failed to create directory `//lib/python3.13/site-packages/`: Read-only file system (os error 30)

$ uv pip install --python simulated-venv/bin/python --no-build click
Using Python 3.13.2 environment at: /
Resolved 1 package in 1ms
error: Failed to install: click-8.4.2-py3-none-any.whl (click==8.4.2)
  Caused by: Failed to create directory `//lib/python3.13/site-packages/`
  Caused by: failed to create directory `//lib/python3.13/site-packages/`: Read-only file system (os error 30)
```

This reproduces the two material observations: the selected environment becomes `/`, and the wheel
installer targets `//lib/python3.13/site-packages` rather than the selected venv. The trailing
separator and OS error are host-specific, so the native Windows report has a trailing backslash and
error 53.

Existing repository coverage does not exercise this path. The
`crates/uv/tests/python/python_install.rs::python_install_pyodide` integration test is guarded by
`#[cfg(unix)]`; it installs Pyodide, creates a venv, and runs its Python, but never runs
`uv pip install` against it. The Windows integration workflow introduced by astral-sh/uv#17658
creates and activates a Pyodide venv before installing NumPy, so `VIRTUAL_ENV` is present. It does
not pass either the venv directory or its executable through `--python`.

## Related

- astral-sh/uv#17658 (pull request, merged), “Support Pyodide interpreter in windows” — enabled the
  Windows Pyodide workflow, but its package-install step uses an activated environment and therefore
  does not cover either explicit `--python` form.
- astral-sh/uv#12729 (issue, closed), “Pyodide support?” — the original umbrella request for
  Pyodide environments and wheel installation.
- astral-sh/uv#19963 (issue, closed), “Cross-building win->linux using explicit
  `--python-platform=` installs to wrong site-packages dir” — related path-layout behavior, but it
  uses `--prefix` and `--python-platform` rather than selecting a uv-created virtual environment.

## Search evidence

The relevant integration suites under `crates/uv/tests/` and `crates/uv-client/tests/it/` were
searched for Pyodide and Emscripten coverage. Only the Unix-only Python-install integration test
described above covers creation of a Pyodide venv; there is no existing explicit-selector pip
installation test. The changed files and Windows workflow patch in astral-sh/uv#17658 were also
checked to distinguish its activated-environment coverage from this report.
