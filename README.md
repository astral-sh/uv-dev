# Windows: provide a pythonw.exe launcher for uv-managed Python installs

Issue: astral-sh/uv#23131

Classification: enhancement

## Summary

The reporter requests a Windows `pythonw.exe` launcher alongside the console launchers published by `uv python install --default`. Their example installs Python 3.15 and receives `python.exe`, `python3.exe`, and `python3.15.exe`, while `Get-Command pythonw` reports that the command is not recognized. The desired invocation is `pythonw -m my_gui_app`, without allocating a console window.

Reported environment: Windows 11 Pro x64, uv 0.13.6 and 0.13.7, and uv-managed CPython 3.15.2. These version-specific observations were supplied by the reporter; no Windows reproduction was performed.

The closest historical work is astral-sh/uv#19226 and astral-sh/uv#19235. They corrected an existing virtual environment launcher's window mode and interpreter target. They did not add a managed Python launcher to PATH. Current source independently confirms the missing installation target.

## Draft response

`uv python install --default` currently publishes console launchers only, so your report matches the implementation. The windowed trampoline support already exists: astral-sh/uv#19235 fixed virtual environments' launcher mode and `pythonw.exe` target, but did not add a launcher to PATH.

This is an enhancement to the install command. Before implementation, we should settle how `pythonw.exe` follows `--default`, interpreter upgrades, and uninstall. In the meantime, you can use a uv-created environment's `Scripts\pythonw.exe` to run your GUI module.

## Classification

The request adds a Windows GUI launcher to the executables published by uv python install --default. Source confirms that installation currently enumerates console executable names and uses PythonExecutable::console, while virtual environment creation separately supports a windowed pythonw.exe target. No existing issue or PR was found tracking this additional PATH capability, and the historical venv fix did not implement it, so this is not a duplicate or an established regression.

This requests an additional installed entry point and its lifecycle behavior. It does not report that an existing PATH `pythonw.exe` launcher stopped working. The separate virtual environment bug was fixed on August 11, 2026, before the reported uv versions; the new report does not establish a return of that bug.

## Related

- astral-sh/uv#19226 — uv venv creates the same python.exe and pythonw.exe (issue, closed). Tracks managed Windows virtual environments whose existing pythonw.exe launcher incorrectly opened a console. Fixed by astral-sh/uv#19235. It concerns launcher behavior inside a venv, whereas this request adds a missing launcher to PATH.
- astral-sh/uv#19235 — Fix pythonw launcher mode for managed Windows venv links (pull request, merged). Merged August 11, 2026; fixes astral-sh/uv#19226 by selecting a windowed trampoline and targeting the managed pythonw.exe. Its diff leaves Python installation PATH launchers explicitly console-only, so it supplies relevant machinery without implementing this request.

## Supporting evidence

- `crates/uv-python-commands/src/install.rs:984`: `create_bin_links` selects minor-version, major-version, and unversioned executable names when default links are requested. At line 1016 it creates their launchers with `PythonExecutable::console` and selects `installation.executable(false)` for the direct interpreter target. There is no windowed installation target in this list.
- `crates/uv-python-types/src/installation_key.rs:120`: the executable naming methods generate the implementation's minor-version, major-version, and unversioned names; they do not generate a CPython `pythonw.exe` alias.
- `crates/uv-python-managed/src/managed.rs:389`: `ManagedPythonInstallation::executable(true)` supports selecting the underlying windowed executable on Windows. At line 979, `PythonExecutable::windowed` sets `WindowMode::Windowed`; at line 1008 the Windows launcher builder receives the target path and that mode.
- `crates/uv-virtualenv/src/virtualenv.rs:305`: the managed minor-version-link branch constructs a separate `pythonw.exe` target and uses `PythonExecutable::windowed`. This supports the reporter's point that relevant trampoline machinery exists.
- `docs/concepts/python-versions.md:143`: the installing-executables documentation describes `--default` as adding `python` and `python3` executables. It does not describe a published `pythonw` executable.
- The merged diff of astral-sh/uv#19235 switches venv windowed launchers to the windowed interpreter target while explicitly retaining console-mode calls in the Python installation command. The issue comments and merged diff establish the historical fix's scope; the PR description uses earlier API names, so the final diff and current source were used for API details.

The report's broader claims about all other Python distributions and exact runtime DLL placement were not independently verified and are not needed for this classification.

## Search scope and other candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using pythonw, pythonw.exe, pythonw3, Get-Command, not recognized, and uv python install --default. Separately searched installation/PATH aliases and GUI behavior using windowless, windowed, GUI interpreter, console, launcher, shim, and trampoline, including area:windows and searches without version or platform restrictions. Checked closed installation reports and followed comments to merged fixes: astral-sh/uv#19226 to astral-sh/uv#19235, astral-sh/uv#6805 to astral-sh/uv#9152, and astral-sh/uv#6801 to astral-sh/uv#11786. Keyword PR searches returned no direct matches; referenced PRs were inspected directly, including the diff of astral-sh/uv#19235. Ruled out astral-sh/uv#19226 as a duplicate or regression because it fixes an existing venv launcher, not a missing PATH launcher. Also inspected astral-sh/uv#12777, astral-sh/uv#15581, astral-sh/uv#13565, and astral-sh/uv#19156; these concern venv console aliases, console defaults, PyPy names, or GUI window focus. No existing discussion tracking the same PATH launcher request was found.

astral-sh/uv#6805 and its merged implementation astral-sh/uv#9152 provide `uv run --gui-script`, which selects a windowed interpreter for a script. astral-sh/uv#6801 and its merged implementation astral-sh/uv#11786 provide `uvw.exe`, which starts uv without a console window. Both discussions concern running scripts through uv rather than publishing a managed interpreter's `pythonw.exe` on PATH.

astral-sh/uv#15581 was resolved by using `uv python install --default` for console aliases; the new report already uses that option. astral-sh/uv#12777 concerns versioned console aliases inside virtual environments. astral-sh/uv#13565 concerns PyPy-specific PATH names. astral-sh/uv#19156 concerns GUI window focus when launching through a batch file. None tracks the requested missing windowed PATH launcher.

## Maintainer next step

Decide the relationship between `pythonw.exe` and `--default`, including which interpreter owns the unversioned launcher and how upgrades, replacement protection, and uninstall handle it. Any implementation should select both the windowed trampoline mode and the managed windowed interpreter target. The existing venv path provides an implementation reference; this handoff does not establish the size of the required change.

Only this temporary handoff was updated. No repository files or GitHub objects were changed, and no builds or tests were run for this triage.
