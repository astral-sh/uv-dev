# "uv install" creates python*.exe shims to .local/bin regardless of UV_PYTHON_BIN_DIR or UV_TOOL_BIN_DIR

Issue: astral-sh/uv#23300

Original classification: duplicate of astral-sh/uv#13977. The edited report needs reproduction before reclassification.

## Summary

On Windows 11 x86_64 with uv 0.13.1–0.13.9 and Python 3.11.14, the reporter runs
`uv python install 3.11 --default` after configuring uv and its managed Python installations on
`D:`. The uv executable and the Python installation go to their configured directories, but
the reporter sees `python.exe`, `python3.exe`, and `python3.11.exe` in `~/.local/bin` instead
of `D:\uv\bin`. The original PowerShell snippet omitted `UV_PYTHON_BIN_DIR`; the edited snippet
sets it to `D:\uv\bin` before running the command. The reporter also sees `python3.11.exe`
in the same location without `--default`. The title mentions `UV_TOOL_BIN_DIR`, but neither
snippet sets it, and it does not control Python shim placement.

astral-sh/uv#13977 answers the same launcher-directory question. astral-sh/uv#15954 documents
the separate settings, and astral-sh/uv#11230 applies them to a broader Windows D: drive setup.

## Reproduction status

A maintainer asked whether the shim in `~/.local/bin` was left over from an install before
`UV_PYTHON_BIN_DIR` was set. The discussion does not establish whether a run with the override
created a new shim there, whether the old shim was already present, or whether a shim was also
created in `D:\uv\bin`. The leftover-shim explanation is unconfirmed.

On an affected Windows setup, set `$env:UV_PYTHON_BIN_DIR = "D:\uv\bin"` and record the output
of `uv python dir --bin` in the same PowerShell session. Compare the contents of `D:\uv\bin`
and `$HOME\.local\bin` before and after `uv python install 3.11 --default`, ideally in a clean
Windows test environment with no pre-existing Python shims. Record the install output and which
files are newly created. Seeing an old shim remain in the default directory does not establish
that the install ignored the override.

## Classification

The original report matched astral-sh/uv#13977: setting `UV_INSTALL_DIR` moves uv itself, not
its Python launchers. The prior report used Linux symlinks rather than Windows `.exe` launchers,
but both use the same directory selection. A fresh shim created in `~/.local/bin` despite a
valid `UV_PYTHON_BIN_DIR` would be a distinct correctness issue, not answered by the earlier
thread. The edited report raises that possibility but does not yet establish a bug or regression
in uv 0.13.1–0.13.9.

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
