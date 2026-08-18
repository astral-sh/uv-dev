# uv pip install --python <venv> fails for a cpython-*-emscripten-wasm32-musl venv on Windows — malformed path mixing POSIX and Windows conventions

Issue: astral-sh/uv#21199

Classification: bug

## Summary

On Windows with uv 0.12.5, the reporter creates a virtual environment with
`uv venv myvenv --python cpython-3.13.2-emscripten-wasm32-musl`. The resulting environment has a
Windows host layout, including `myvenv\Lib\site-packages` and `myvenv\Scripts\python.exe`.
However, `uv pip install --python myvenv --no-build click` reports that it is using the environment
at `/` and attempts to create `//lib/python3.13/site-packages\`. Windows interprets that malformed
mixed-separator path as a network path and returns OS error 53. Passing
`myvenv\Scripts\python.exe` to `--python` produces the same result.

Windows Pyodide package installation is intended to work: merged astral-sh/uv#17658 explicitly
enabled it and added integration coverage for creating and activating a Pyodide venv before
running `uv pip install packaging`. That test did not exercise either explicit `--python` form in
this report. No existing open issue or pull request was found for the `/` environment root or the
malformed `//lib/python3.13/site-packages\` destination.

## Draft response

Thanks for the clear reproduction. Windows + Pyodide package installation is intended to work;
astral-sh/uv#17658 added support and covered installation into an activated uv-created venv. This
report exposes a different path: explicitly selecting that venv, either by directory or by
`Scripts\python.exe`, resolves the environment as `/` and produces a destination that is not valid
for the Windows venv.

This is different from the intentional `--prefix` plus `--python-platform` behavior discussed in
astral-sh/uv#19963, so this should be treated as a bug. The next useful step is to reproduce both
explicit `--python` forms against `main` and extend the Windows Pyodide integration coverage from
astral-sh/uv#17658 to cover them.

## Classification

This is a bug because two supported uv operations disagree about the same environment: `uv venv`
creates it successfully, while `uv pip install --python` resolves its root as `/` and attempts to
write through an invalid UNC-like path. The current source also corroborates the important values
in the report: uv's Pyodide interpreter fixture models `sys_prefix` as `/`, an absolute
`//lib/pythonX.Y/site-packages` install scheme, and a relative
`lib/pythonX.Y/site-packages` virtualenv scheme. The report establishes that the explicit discovery
path is retaining the former instead of targeting the venv that the user selected. A final code
diagnosis still requires a Windows reproduction, but the emitted destination is incorrect
regardless of the precise point where the wrong scheme is retained.

This is not a duplicate. astral-sh/uv#17658 is a merged support change rather than an open tracker
for this explicit-selection failure. astral-sh/uv#19963 uses `--prefix` and
`--python-platform` without an existing virtual environment, and maintainers documented its host
layout as intended under that API. The broader open astral-sh/uv#16023 concerns Pyodide discovery
names, while astral-sh/uv#15709 concerns `UV_PYTHON`/`--python` selection semantics for a Nix system
interpreter; neither has the malformed venv root or installation destination reported here.

## Related

- astral-sh/uv#17658 (pull request, merged), “Support Pyodide interpreter in windows” — the closest
  implementation history. It explicitly enabled Windows + Pyodide `uv pip` and tested package
  installation after activating a uv-created venv. Its test plan does not cover
  `uv pip install --python <venv>` or selecting the venv executable directly, which is the key
  difference in astral-sh/uv#21199.
- astral-sh/uv#12729 (issue, closed), “Pyodide support?” — the original umbrella request asked for
  both Pyodide virtual environments and Pyodide wheel installation. Its discussion explicitly
  described Windows support as an open question before the later work in astral-sh/uv#17658.
- astral-sh/uv#19963 (issue, closed), “Cross-building win->linux using explicit
  `--python-platform=` installs to wrong site-packages dir” — adjacent because it also reports a
  host/target layout mismatch. It is not the same behavior: it installs with `--prefix` and
  `--python-platform` rather than selecting a real uv-created venv, and maintainers confirmed that
  its host-layout behavior is intentional for those APIs.

## Search evidence

Literal searches covered `//lib/python3.13/site-packages`, “network path was not found,”
“environment at: /,” `emscripten-wasm32-musl`, and the `Lib/site-packages`/`Scripts` layout. No
earlier issue or pull request contained the same failure. Conceptual searches covered Pyodide and
Emscripten support, Windows virtual environments, `--python` destination inference, sysconfig and
install schemes, POSIX/Windows path mixing, and cross-platform installation layouts. Fix-oriented
searches covered closed issues and merged pull requests, leading to astral-sh/uv#12729 and its
initial Pyodide work, then the closer Windows support change astral-sh/uv#17658. The reporter's
astral-sh/uv#19963 lead was inspected with its maintainer comments and ruled out as a duplicate for
the API and expected-layout differences described above.
