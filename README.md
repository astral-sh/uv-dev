# Fails to init when parent folder name is "python"

Issue: astral-sh/uv#21393

Classification: bug

## Summary

Running `uv init --python 3.13` from a directory named `Python` infers the normalized project name
`python`. The current default packaged-application template uses the project name for both the
package and its console entry point, so it generates `[project.scripts] python`. A later project
sync rejects that entry point because `python` is reserved for the environment's interpreter.

The invalid generated project is reproducible, but the report's claim that `uv init` itself emits
the installation error is not. With the installed uv 0.12.8, `uv init` succeeds and returns before
any environment creation or project installation. A separate `uv sync` emits the exact reported
error. The reporter's exact uv version and complete command output are needed to explain that
command-boundary discrepancy on Windows 11.

The repository history establishes how these behaviors intersect. astral-sh/uv#12983 reported the
same directory-derived `python` entry point corrupting the virtual environment during `uv sync`.
astral-sh/uv#13051 fixed that dangerous behavior by rejecting scripts that would overwrite
`python`, producing the exact reserved-name error shown here. astral-sh/uv#19197 subsequently made
packaged applications with project-named entry points the stable `uv init` default, exposing the
collision without an explicit `--package` option. No open issue or pull request found in the search
already tracks the resulting early-validation and error-message gap.

## Reproduction

Outcome: **not reproducible as reported**. The reported `uv init` failure was tested on Linux
6.17.0-1022-azure x86_64 with uv 0.12.8. All files, uv cache data, and managed Python installations
were isolated in a new runner temporary directory, and configuration discovery was disabled.

From a newly created directory named `Python`:

```console
$ uv --version
uv 0.12.8 (x86_64-unknown-linux-gnu)
$ uv init --python 3.13
Initialized project `python`
$ echo $?
0
```

Initialization created `.python-version` containing `3.13` and a `pyproject.toml` containing:

```toml
[project]
name = "python"

[project.scripts]
python = "python:main"
```

It also created the other normal project files (`README.md`, `.gitignore`, `.git`, and
`src/python/__init__.py`). It did not create `.venv`, build a wheel, or print an installation
error. Running the first sync separately with an explicit Python 3.13 reproduced the report's
installer failure:

```console
$ uv sync --python 3.13
Using CPython 3.13.15
Creating virtual environment at: .venv
...
error: Failed to install: python-0.1.0-py3-none-any.whl (python==0.1.0 (...))
  Caused by: Scripts must not use the reserved name `python`, got: `python`
$ echo $?
2
```

The naming workaround was also verified: `uv init --name python-project --python 3.13` followed by
`uv sync --python 3.13` both exited successfully.

Existing integration tests cover the two component behaviors, but no test covers the exact
directory-name-to-sync sequence end to end:

- `crates/uv/tests/project/init.rs`, test `init`, asserts that default initialization succeeds,
  writes a project-named `[project.scripts]` entry, and does not install the project (its subsequent
  check is `uv lock`).
- `crates/uv/tests/pip_install/pip_install.rs`, test `reserved_script_name`, installs a local project
  whose script is named `python` and asserts the same exit-code-2 reserved-name diagnostic.

To reproduce the failure specifically within `uv init` on the reported Windows 11 system,
maintainers still need the exact output of `uv --version`, the complete output from a clean
`uv init --verbose --python 3.13`, and confirmation of whether an IDE, shell integration, wrapper,
or follow-up command automatically ran a sync after initialization.

## Draft response

Thanks for the report. We can reproduce that initialization in a directory named `Python` creates
a project with a reserved `python` entry point, and that the first `uv sync` rejects the generated
wheel with the error shown. uv deliberately rejects that entry-point name because it would
overwrite the environment's Python executable; that protection was added in astral-sh/uv#13051 for
astral-sh/uv#12983. Since astral-sh/uv#19197 made packaged applications the default, `uv init`
should avoid generating this unusable default or detect the collision with an actionable message.

As a workaround, please use a non-reserved project name, for example
`uv init --name my-project --python 3.13`. In our test, however, `uv init --python 3.13` itself
succeeded and only a separate sync produced the installation error. Could you provide the exact
output of `uv --version`, the complete output of `uv init --verbose --python 3.13` in a clean
directory, and whether an IDE or shell integration might run a sync automatically? Those details
are needed to reproduce the reported failure specifically within `uv init` on Windows 11.

## Classification

This is a bug in the generated default project, although the reported failure within `uv init` was
not reproduced. Direct observation confirms that `uv init` derives `python` from the target
directory and creates a console entry point with that name. The generated project cannot be
installed as-is because the wheel installer intentionally rejects `python` as a reserved script
name. uv should avoid generating this unusable default or diagnose the collision clearly.

This should not be centralized as a duplicate of closed astral-sh/uv#12983. That issue tracked
virtual-environment corruption and was fixed by the still-effective rejection in
astral-sh/uv#13051. The new issue concerns the current default init template, early validation, and
diagnostic behavior after astral-sh/uv#19197 made packaged applications the default. It is also not
a regression of the old corruption: the safeguard is preventing the destructive overwrite as
designed.

The exact claim that project installation occurs within `uv init` conflicts with the reproduction:
initialization exited successfully after generating files, while the separately invoked sync
failed. The requested exact version and complete Windows output are therefore needed to establish
that part of the report without weakening the confirmed invalid-template bug.

## Related

- astral-sh/uv#12983 — **uv sync breaks venv if folder is called python** (closed). This is the
  closest historical issue: it has the same `python` directory, inferred project name, and generated
  `[project.scripts] python` collision. Its actual result was environment corruption during
  `uv sync`, whereas astral-sh/uv#21393 observes the later safety check rejecting installation.
- astral-sh/uv#13051 — **Block scripts from overwriting `python`** (merged). This fixed
  astral-sh/uv#12983 by reserving the interpreter's script names. It accounts for the exact
  `Scripts must not use the reserved name` error and confirms that bypassing the check is not the
  appropriate fix.
- astral-sh/uv#19197 — **Package by default: Stabilize preview feature** (merged). This changed the
  stable default for `uv init` to a packaged application with an entry point named after the
  project. It explains why an ordinary init now generates the conflicting entry point without an
  explicit `--package` option.
- astral-sh/uv#16554 — **Improve `uv init` error for invalid directory names** (merged). This is an
  adjacent error-message precedent: it makes failures from inferred directory names identify the
  directory and recommend `--name`. It does not cover this case because `python` is a valid package
  name; the generated console script is what is invalid.

## Search evidence

Literal searches covered the exact reserved-name error, the
`python-0.1.0-py3-none-any.whl` identifier, `python` folder and directory wording, and `uv init`.
Conceptual searches separately covered inferred project names, generated console entry points,
project-name validation, partial initialization, cleanup, atomic or transactional init behavior,
and the package-by-default change. Searches included open and closed issues and open, closed, and
merged pull requests. Fix-oriented inspection followed astral-sh/uv#12983 to its closing pull
request astral-sh/uv#13051, then inspected the later default change in astral-sh/uv#19197 and the
analogous inferred-name diagnostic in astral-sh/uv#16554.

astral-sh/uv#5152 and its fix astral-sh/uv#5165 were plausible historical leads because a package
entry point named `python` overwrote a symlink target. They are less direct than astral-sh/uv#12983:
they concern atomic entry-point writes for an explicitly configured script and do not address
directory-name inference, the default init template, or early validation.
