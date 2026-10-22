# Python 3.14 NLTK Installation Error on Windows 11

Issue: astral-sh/uv#22845

Classification: question

## Summary

The reporter uses Windows 11 x86_64, Python 3.14, VS Code, and uv 0.12.17
(635500036, 2026-09-18). Installing NLTK with pip produces
`error: externally-managed-environment` and says the Python installation is managed by uv.
Running their Python file produces `ModuleNotFoundError: No module named 'nltk'`;
the reporter attributes this to installation in a different environment. They ask how to
install packages and configure VS Code.

The issue was opened on 2026-10-22 and has no comments. It does not provide the exact
installation command, full traceback, selected interpreter path, virtual-environment setup,
or script-launch method.

astral-sh/uv#12204 explains the restriction introduced by astral-sh/uv#4312. astral-sh/uv#12604 and astral-sh/uv#9637 cover adjacent pip and VS Code environment confusion; no confirmed duplicate or regression was established.

## Draft response

uv-managed base Python installations are intentionally protected from package installs (see astral-sh/uv#12204). Install NLTK into a virtual environment, then select that same environment in VS Code.

From your project folder in PowerShell:

```powershell
uv venv --python 3.14
uv pip install --python .venv\Scripts\python.exe nltk
.\.venv\Scripts\python.exe -c "import sys, nltk; print(sys.executable); print(nltk.__version__)"
```

In VS Code, open that project folder, then use **Python: Select Interpreter** from the Command Palette and select `<project>\.venv\Scripts\python.exe`. Use **Enter interpreter path** if it is not listed. Run your script with **Python: Run Python File in Terminal**.

The commands above do not require activation or pip installed in the virtual environment. If the import still fails, please share the full command output, the interpreter path selected in VS Code, and how you launch the script.

## Classification

The report asks for package-installation and VS Code setup guidance. Source and maintainer comments confirm that the base-interpreter restriction is intentional; incorrect uv behavior has not been established. Related discussions concern deliberate global installation, pip fallback from an activated virtual environment, or editor failure after selecting an interpreter, none of which is established as the same underlying problem here. There is no evidence of a Python 3.14 or uv 0.12.17 regression.

The existing global-installation questions ask to modify or share a base interpreter.
This reporter asks how to configure ordinary package installation and script execution, without
requesting global installation. The pip-fallback discussion requires a virtual environment without
pip, and the editor discussion describes failure after interpreter selection. Neither triggering
condition is documented here, so those discussions are supporting context rather than an established
duplicate.

A missing import alone does not establish an installer defect. The reported environment mismatch
has not been independently verified. The recommended commands target one interpreter explicitly and
provide a direct import check; any remaining failure would need its full output and the VS Code
interpreter and launch method.

## Related

- astral-sh/uv#12204 — `uv pip install --system` fails because python installation is managed by uv? (open; issue). Reports the identical uv-managed-installation message. A maintainer confirms that protecting the base environment is intentional and recommends a virtual environment. Its request concerns global installation for multiple users, unlike this project and editor setup question.

- astral-sh/uv#12604 — Improve ux when `pip` is not present in virtual environments (open; issue). Canonical discussion of bare pip falling back to another environment when a uv-created virtual environment lacks pip. Relevant to the installation/import mismatch, but this report does not establish an activated virtual environment or that fallback mechanism.

- astral-sh/uv#9637 — Selecting .venv/bin/python as an active interpreter in VSCode doesn't enable IDE features (closed; issue). Discusses VS Code interpreter selection and missing imports, with maintainer guidance on editor configuration. Unlike this report, it describes failure after explicitly selecting the virtual environment; it does not establish a shared editor defect here.

- astral-sh/uv#4312 — Add `EXTERNALLY-MANAGED` markers to managed toolchains (merged; pull request). Merged on 2024-06-17, closing astral-sh/uv#4240 and intentionally protecting managed base installations, including Windows. This explains the restriction; it is not a fix for an NLTK installation regression.

## Supporting evidence

- `crates/uv-python-managed/src/managed.rs:320` contains the exact message
  “This Python installation is managed by uv and should not be modified.”
  `ManagedPythonInstallation::ensure_externally_managed`, starting at line 570, writes the
  `EXTERNALLY-MANAGED` marker into the managed interpreter's standard-library directory,
  using `Lib` on Windows.
- `crates/uv-python-interpreter/src/interpreter.rs:306` implements
  `Interpreter::is_externally_managed`; it explicitly exempts virtual environments.
  This establishes uv's own behavior, not which executable the reporter's pip invocation used.
- `docs/pip/environments.md` documents creating a virtual environment with
  `uv venv --python`, installing with `uv pip install`, and explicitly targeting an
  interpreter with `--python`. The draft uses an explicit Windows interpreter path for both
  installation and the import check.
- In astral-sh/uv#12204, maintainer zanieb's 2025-03-16 comment confirms that the global managed
  environment is intentionally not mutable and recommends a virtual environment.
  The historical policy was implemented by astral-sh/uv#4312 on 2024-06-17, well before the
  reported uv release.
- In astral-sh/uv#6593, maintainer charliermarsh explains that uv does not install pip in virtual
  environments by default, so a bare pip command can resolve to a different installation.
  The consolidated discussion is astral-sh/uv#12604. This is a possible explanation requiring
  confirmation, not an established cause for this report.
- astral-sh/uv#10300 documents the same managed-base restriction and a working solution that
  creates a virtual environment, installs into it explicitly, and points the editor at its Python.
  That case concerns Neovim's Python provider rather than running a Python file in VS Code.
- astral-sh/uv#7803 contains successful reports using VS Code's
  “Python: Select Interpreter” with the project virtual environment.
  astral-sh/uv#9637 covers actual selection failures and is labeled external; maintainer konstin
  closed it on 2026-02-23 with configuration guidance. The present report does not say selection
  of a project virtual environment was attempted or rejected.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separately searched installation errors (externally-managed-environment, EXTERNALLY-MANAGED, the exact managed-installation message, nltk) and import/editor symptoms (ModuleNotFoundError, VS Code, vscode, Select Interpreter). Broadened to externally managed, wrong/different environment, pip fallback, virtual environments, and interpreter selection without package/version/platform constraints. Checked closed issues for Python 3.14 and uv 0.12.17 and PR searches for managed environments, interpreters, pip/venv, and 3.14. Keyword PR searches returned no candidates; reference chains identified merged astral-sh/uv#4312 and astral-sh/uv#7934. Read candidate discussions and followed astral-sh/uv#12604 to astral-sh/uv#6593, and astral-sh/uv#16454 to astral-sh/uv#9637. Ruled out astral-sh/uv#18639 (Windows 11/Python 3.14, but VS Code rejects a working interpreter), astral-sh/uv#16198 (Poetry/virtualenv-specific failure), and astral-sh/uv#7934 (uv interpreter-preference validation, not standalone pip). No matching regression fix was found.

Additional inspected candidates help distinguish similar errors:

- astral-sh/uv#18639 reports “Selected file is not a valid Python interpreter” on Windows 11 with
  Python 3.14.3, while scripts already run successfully. That is a different observable failure
  from pip refusing installation and a missing third-party import.
- astral-sh/uv#16198 involves Poetry install/sync invoking pip inside an apparent virtual
  environment; the reporter traced it to a Poetry/virtualenv issue and a Poetry version difference.
  No Poetry workflow is reported here.
- astral-sh/uv#15334 and astral-sh/uv#15517 concern tox/pre-commit and failure to locate the
  standard-library `encodings` module. Maintainers identify a python-build-standalone issue;
  those symptoms and tools are absent here.
- astral-sh/uv#10022 and astral-sh/uv#15635 were resolved by removing `--system`, which
  intentionally bypasses virtual environments. No such uv command is supplied here.
- astral-sh/uv#11729 alleges an externally-managed error during `uv sync` with
  `UV_PROJECT_ENVIRONMENT`; it closed without the requested follow-up. This report uses pip
  and does not identify project-environment overrides.
- astral-sh/uv#7934, linked from astral-sh/uv#6067, merged on 2025-07-16 and validates uv's
  interpreter preferences. Its fix predates uv 0.12.17, but the report supplies no uv discovery
  command or preference setting that would establish a regression of it.
- Other inspected editor/environment reports, including astral-sh/uv#8964,
  astral-sh/uv#8706, astral-sh/uv#3037, astral-sh/uv#13793, and astral-sh/uv#13101,
  concern discovery, terminal integration, shell-specific configuration, or isolated tool
  environments. None establishes the same failure here.

## Validation and next step

This is an evidence-based support assessment, not a Windows reproduction. No package installation,
build, or integration test was run. The proposed response asks the reporter to install and import
NLTK using one explicit virtual-environment interpreter, then select that interpreter in VS Code.
If it fails, the command output and editor launch details will distinguish an installation problem
from an interpreter-selection problem.

The only authored artifact is this handoff. No GitHub changes were made. The checkout was not
modified; its observed pre-existing status lists `agents/codex/config.toml` as modified and
`.issue-triage-event.json` as untracked.
