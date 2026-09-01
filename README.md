# Fails to init when parent folder name is "python"

Issue: astral-sh/uv#21393

Classification: bug

## Summary

Running `uv init --python 3.13` from a directory named `Python` infers the normalized project name
`python`. The current default packaged-application template uses the project name for both the
package and its console entry point, so it generates `[project.scripts] python`. Wheel installation
then rejects that entry point because `python` is reserved for the environment's interpreter. The
reporter sees the rejection only after initialization files have been created and asks for the
collision to be detected earlier with an actionable diagnostic.

The repository history establishes how these behaviors intersect. astral-sh/uv#12983 reported the
same directory-derived `python` entry point corrupting the virtual environment during `uv sync`.
astral-sh/uv#13051 fixed that dangerous behavior by rejecting scripts that would overwrite
`python`, producing the exact reserved-name error shown here. astral-sh/uv#19197 subsequently made
packaged applications with project-named entry points the stable `uv init` default, exposing the
collision without an explicit `--package` option. No open issue or pull request found in the search
already tracks the resulting early-validation and error-message gap.

## Draft response

Thanks for the report. The inferred project name `python` is valid as a package name, but the
default packaged-application template also creates a `python` entry point. uv deliberately rejects
that entry-point name because it would overwrite the environment's Python executable; that
protection was added in astral-sh/uv#13051 for astral-sh/uv#12983. Since astral-sh/uv#19197 made
packaged applications the default, `uv init` should detect this collision before writing project
files and explain which inferred name caused it.

As a workaround, please use a non-reserved project name, for example
`uv init --name my-project --python 3.13`. Could you also provide the exact output of `uv --version`
and `uv init --verbose --python 3.13`? Current source does not perform the project installation
inside `uv init`, so that detail is needed to confirm where the reported install step is being
triggered.

## Classification

This is a bug. Source inspection confirms that `uv init` derives the project name from the target
directory when `--name` is absent, and that the default `ApplicationWithLibrary` template creates a
console entry point with that same name. Source and tests also confirm that wheel installation
intentionally rejects `python` as a reserved script name. The combination therefore produces a
default-generated project that cannot be installed as-is, while the reported ordering leaves
partial initialization files and reports a low-level installation error instead of identifying the
inferred-name conflict.

This should not be centralized as a duplicate of closed astral-sh/uv#12983. That issue tracked
virtual-environment corruption and was fixed by the still-effective rejection in
astral-sh/uv#13051. The new issue concerns the current default init template, early validation, and
diagnostic behavior after astral-sh/uv#19197 made packaged applications the default. It is also not
a regression of the old corruption: the safeguard is preventing the destructive overwrite as
designed.

The exact claim that the project installation occurs within `uv init` is not explained by the
current command implementation, which generates files but does not sync the project. The requested
exact version and verbose output are therefore genuinely needed to establish that part of the
reproduction without weakening the source-confirmed template conflict.

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
