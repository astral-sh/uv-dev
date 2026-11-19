# --default creates python*.exe to .local/bin instead of UV_TOOL_BIN_DIR

Issue: astral-sh/uv#23300

Classification: duplicate

## Summary

On Windows 11 x86_64 with uv 0.13.1–0.13.9 and Python 3.11.14, the reporter runs
`uv python install 3.11 --default` after configuring uv and its managed Python installations on
`D:`. The uv executable and the Python installation go to their configured directories, but
`python.exe`, `python3.exe`, and `python3.11.exe` go to `~/.local/bin` instead of `D:\uv\bin`.
The title mentions `UV_TOOL_BIN_DIR`; the PowerShell snippet sets `UV_INSTALL_DIR` and
`UV_PYTHON_INSTALL_DIR`, but shows neither `UV_TOOL_BIN_DIR` nor `UV_PYTHON_BIN_DIR`.

astral-sh/uv#13977 answers the same launcher-directory question. astral-sh/uv#15954 documents
the separate settings, and astral-sh/uv#11230 applies them to a broader Windows D: drive setup.

## Draft response

`uv python install --default` uses `UV_PYTHON_BIN_DIR` for the Python launchers.
`UV_INSTALL_DIR` controls where the standalone installer puts `uv` and `uvx`,
`UV_PYTHON_INSTALL_DIR` controls where the interpreters are stored, and `UV_TOOL_BIN_DIR` is
for commands installed by `uv tool install`.

In the same PowerShell session, set `$env:UV_PYTHON_BIN_DIR = "D:\uv\bin"`, check that
`uv python dir --bin` prints that directory, then rerun `uv python install 3.11 --default`.
The same launcher-directory question was answered in astral-sh/uv#13977.

## Classification

Duplicate of astral-sh/uv#13977. Both reports use `UV_INSTALL_DIR` to relocate uv and then
expect `uv python install --default` to put the Python launchers in the same directory instead
of `~/.local/bin`. The prior report used Linux symlinks rather than Windows `.exe` launchers,
but both use the same directory selection. Neither `UV_INSTALL_DIR` nor `UV_TOOL_BIN_DIR`
overrides the Python launcher directory. The reported setup does not show `UV_PYTHON_BIN_DIR`
being ignored, so it does not establish a regression in uv 0.13.1–0.13.9.

## Related

- astral-sh/uv#13977 (closed issue): The same command used `~/.local/bin` despite `UV_INSTALL_DIR` pointing elsewhere. A maintainer identified `UV_PYTHON_BIN_DIR`, and the reporter confirmed that answered the question.
- astral-sh/uv#15954 (merged pull request): Superseded the storage documentation PR referenced in the canonical answer and documented separate locations for uv, managed Python launchers, and tool executables. This was a documentation change, not a runtime fix.
- astral-sh/uv#11230 (open issue): The Windows D: drive discussion explicitly distinguishes `UV_PYTHON_BIN_DIR` from `UV_INSTALL_DIR` and `UV_TOOL_BIN_DIR`. It covers relocating all uv storage rather than this specific `--default` behavior.

## Supporting evidence

- `crates/uv-python-commands/src/install.rs:659` resolves the Python executable directory; `crates/uv-python-commands/src/install.rs:984` shows that `--default` selects additional executable names, not another destination.
- `crates/uv-python-managed/src/managed.rs:1066` passes `UV_PYTHON_BIN_DIR` to the shared resolver. `crates/uv-dirs/src/lib.rs:24` checks that override, then the XDG settings, then falls back to `~/.local/bin` on all platforms.
- `crates/uv-python-commands/src/dir.rs:12` uses the same resolver for `uv python dir --bin`. `crates/uv-tool/src/lib.rs:395` passes `UV_TOOL_BIN_DIR` only for tool executables. `docs/reference/storage.md:138` documents these directories and the standalone installer setting separately.

## Search scope

Used `gh` to search open and closed issues and open, closed, and merged PRs for `--default`,
`python3.11.exe`, `.local/bin`, and the directory variables. Broadened the search to shims,
symlink location, launchers, and executable directories without platform or version terms.
Separately checked configuration and fallback reports, the documentation PR referenced by
astral-sh/uv#13977 and its successor, and fixes from the reported 0.13 series.
astral-sh/uv#14342 already set the Python bin override and concerned its availability to the
running process; astral-sh/uv#13979 requests a new CLI flag. Neither is the reported behavior.
astral-sh/uv#22547 fixes broken links for relative interpreter installation paths, while
astral-sh/uv#14261 fixes which interpreter existing launchers target on upgrade; neither
changes where `uv python install --default` places the launchers.
