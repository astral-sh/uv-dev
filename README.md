# UV run package conflicts with binaries installed on the operating system

Issue: astral-sh/uv#21238

Classification: question

## Summary

The reporter said that “UV run logger” in a uv workspace executed `/usr/bin/logger` rather than
their workspace's Python package named `logger`. A maintainer asked them to confirm that the exact
invocation was `uv run logger` and stated that, in general, the workspace should take precedence
over commands on `PATH`. This strongly points to `uv run` command resolution as the relevant
subsystem, but the exact command has not yet been explicitly confirmed.

`uv run` runs a command with the project environment on `PATH`; it does not interpret an arbitrary
command argument as a request to run or import the same-named Python package. A Python distribution
must declare a matching console-script entry point for installation to create a `logger` executable.
If the workspace package does not provide that executable, `uv run` may resolve the existing
`/usr/bin/logger`; astral-sh/uv#3097 tracks a strict mode that would reject commands not provided by
the environment. If the package does declare a `logger` entry point and the workspace environment
was synchronized, selecting `/usr/bin/logger` conflicts with the maintainer's stated precedence and
could be a command-resolution bug.

## Reproduction status

Partial reproduction inferred from the issue author's clarification:

1. Use a uv workspace containing a custom Python package named `logger`.
2. Apparently run `uv run logger`; the maintainer has requested explicit confirmation of this exact
   invocation.
3. Observe that `/usr/bin/logger` is executed.
4. The reporter expected the workspace Python package to run instead.

The report still lacks confirmation of the literal command, the uv and macOS versions, a minimal
workspace, command output, and the package metadata needed to determine whether it provides a
`logger` executable.

## Information needed

- The relevant `pyproject.toml` sections, particularly `[project.scripts]` or other entry-point
  declarations for `logger`.
- Explicit confirmation that the command was `uv run logger`, including the directory from which it
  was run and which workspace member it was intended to target.
- Whether the workspace package is installed in the project environment and whether
  `.venv/bin/logger` exists after `uv sync`.
- The output of `uv --version`, the macOS version, and a verbose invocation such as
  `uv run --verbose logger`.
- Whether the intended operation is to invoke a console script or import/run a Python module. For a
  module, the relevant comparison is `uv run python -m logger`, assuming the package supports module
  execution.

## Classification

`question` remains the best classification pending the requested details. The maintainer has now
established that workspace commands should generally take precedence over `PATH`, so a verified
workspace-provided `logger` executable losing to `/usr/bin/logger` would be incorrect behavior.
However, the report still does not confirm the literal invocation or establish that the Python
package installs a `logger` executable; a package name alone is not a command entry point.

If the package does not provide that executable, astral-sh/uv#3097 already tracks the enhancement
that would prevent commands outside the current environment from being run. If a minimal
reproduction confirms that `.venv/bin/logger` exists but uv still selects `/usr/bin/logger`, the
maintainer's precedence statement supports reclassification as a bug.

## Related

- astral-sh/uv#3097 — Closest existing issue. It tracks a strict `uv run` mode in which only commands
  provided by the current environment may run, avoiding fallback to same-named operating-system
  commands. It would reject a missing workspace entry point rather than infer and run a Python
  package.
- astral-sh/uv#15384 — Discusses the intended distinction between `uv run` for project-context
  commands and `uvx`/`uv tool run` for tools in isolated environments.
- astral-sh/uv#7804 — Similar name-collision symptom for `uvx`, but not the command used here. That
  issue concerned fallback to an unrelated executable on `PATH` when an inferred tool package did
  not provide the requested command.
- astral-sh/uv#11603 — Merged fix for the `uvx` behavior in astral-sh/uv#7804. It does not establish
  equivalent strict command provenance for `uv run`.
