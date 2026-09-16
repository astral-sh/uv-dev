# `uv check` and `ty check` report different errors

Issue: astral-sh/uv#21720

Classification: question

## Summary

With `project.requires-python = ">=3.12"` and a Python 3.14 project environment, standalone
`ty check` and `uv run ty check` accept the example, while `uv check` reports a
`not-subscriptable` diagnostic for `typing.Union`. Changing the requirement to Python 3.14 makes
standalone ty report the diagnostic, and selecting Python 3.12 with `uv check --python 3.12` makes
`uv check` pass. Pinning the same ty release through `--ty-version` does not remove the difference.

The reported behavior is reproducible with the reported uv and ty versions on Linux. Verbose ty
output confirms that standalone ty targets Python 3.12, the lower bound of `requires-python`, while
the uv-integrated invocation targets the Python 3.14 project environment. A maintainer confirmed
that preferring the existing environment is intentional current behavior, while also noting that
the project has been debating whether the safety of checking the full declared Python range should
take precedence. The reporter clarified that the practical impact is a `ty-pre-commit` failure:
the hook runs `uv check`, while a direct `ty check` on the same project passes.

## Reproduction

Outcome: **reproducible**.

The reproduction used Linux x86_64, `uv 0.12.15`, ty 0.0.81, and an isolated temporary directory
and cache. The project contained the reported `demo.py` and this configuration (the dev dependency
only makes the reported direct and `uv run` command forms available at the same pinned ty version):

```toml
[project]
name = "demo"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []

[dependency-groups]
dev = ["ty==0.0.81"]
```

After creating the project environment with `uv sync --python 3.14`, the interpreter was CPython
3.14.7. The targeted commands produced:

```console
$ .venv/bin/ty check
All checks passed!

$ uv run ty check
All checks passed!

$ uv check --ty-version 0.0.81
error[not-subscriptable]: Cannot subscript object of type `<special-form 'typing.Union'>` with no `__class_getitem__` method
  --> demo.py:11:16
Found 1 diagnostic
```

The reported variants also reproduced: `uv check --python 3.12 --ty-version 0.0.81` passed, while
changing `requires-python` to `>=3.14` made standalone ty 0.0.81 emit the same diagnostic. Verbose
output showed `Python version: Python 3.12` for standalone ty and `Python version: Python 3.14`
when `TY_UV=1` enabled uv metadata discovery.

There is no integration test asserting this exact difference in target-version inference.
`crates/uv/tests/workspace/workspace_metadata.rs::workspace_metadata_includes_existing_environment`
does assert that metadata for an existing project environment includes its concrete interpreter
version, while `crates/uv/tests/project/check.rs` exercises `uv check` broadly. The latter does not
compare its target Python version with standalone ty.

## Maintainer assessment

The current preference is intentional and follows the environment-oriented behavior of other uv
commands: `uv check` uses the existing compatible project environment and passes its Python version
to ty. This is useful when dependency selection changes with Python markers, since checking against
the concrete environment keeps type analysis aligned with the packages actually selected there.

The competing goal is minimum-version compatibility. Standalone ty derives its target from the
lower bound of `project.requires-python`, which can catch use of Python or typing features that are
unavailable on versions the project claims to support. The maintainer comment identifies this as a
potentially more important safety property, so the precedence remains a design tradeoff rather than
an accidental implementation difference.

Changing only the Python-version preference would not make `uv check` a drop-in replacement for
`ty check`. The commands also intentionally differ in scope: `uv check` can select multiple
workspace members where `ty check` selects the current project, and `uv check` excludes PEP 723
scripts unless explicitly requested while ty includes them without preparing their environments.

## Reported workflow impact

The reporter is comfortable with the commands having different semantics if the distinction is
clear. Their unexpected failure came from `ty-pre-commit`: the hook invokes `uv check`, so it checks
the concrete project environment and can fail even when a developer's direct `ty check` passes
against the project's minimum declared Python version. They ask whether the hook should invoke ty
directly instead. No maintainer decision on the hook behavior is present in the discussion yet.

## Draft response

`requires-python` is not ignored when uv selects the project interpreter: `>=3.12` permits the
Python 3.14 interpreter in `.venv`. In the reproduced case, `uv check` enables ty's uv integration,
and ty targets that environment's concrete Python 3.14 version. Standalone `ty check` and
`uv run ty check` instead target the lower bound of `project.requires-python`, Python 3.12.
`--ty-version` only selects the ty executable, so it does not affect the target Python version.

To check against Python 3.12 with `uv check`, use `uv check --python 3.12`.

## Classification

This remains classified as a question because a maintainer has confirmed that the
existing-environment preference is intentional current behavior, and the reporter's follow-up asks
which intentional command semantics `ty-pre-commit` should expose. `requires-python` is a
compatibility range, and Python 3.14 satisfies `>=3.12`. The current uv source sets `TY_UV=1` for
`uv check`, and workspace metadata exposes the selected environment's concrete interpreter.
Observed verbose output confirms that ty uses that Python 3.14 version under uv integration, while
standalone ty uses the lower bound of `project.requires-python`.

The repository discussion treats the alternative as an unresolved product tradeoff, not a
confirmed correctness defect: concrete-environment fidelity helps with Python-gated dependencies,
while lower-bound checking provides broader compatibility protection. This is also not a duplicate
of astral-sh/uv#19790: that issue concerns an invalid local-environment error when `/usr/local` is
configured as a system project environment, whereas astral-sh/uv#21720 uses a valid `.venv` and
successfully checks against its concrete Python version.

## Related

- astral-sh/uv#19790 (open issue) — The closest adjacent report: `ty check` and
  `uv run ty check` work while `uv check` changes ty's Python-environment behavior. Its trigger is
  an invalid system-prefix-as-venv path rather than successful selection of a valid newer
  environment, so it is not the same support question.
- astral-sh/uv#19655 (merged pull request) — Introduced the `TY_UV=1` integration switch that
  remains in the current source and causes ty under `uv check` to query uv project metadata.
- astral-sh/uv#20643 (merged pull request) — Added the concrete existing-environment interpreter to
  `uv workspace metadata`, specifically so ty can consume an environment already synchronized by
  `uv check`. Current ty source consumes that interpreter version as uv-supplied configuration,
  directly explaining the reported Python 3.12-versus-3.14 behavior.

## Supporting evidence

The report isolates the selected Python version independently of the ty release: `--python 3.12`
changes the result, while `--ty-version 0.0.81` does not. The CLI documentation and current source
also define `--ty-version` solely as ty executable selection. A follow-up check found that
`[tool.ty.environment] python-version = "3.12"` did not override the uv-supplied Python 3.14 target
in ty 0.0.81, so that previously suggested workaround has been removed; direct
`ty check --python-version 3.12` did pass, but `uv check` does not expose that ty option.

Literal searches covered the complete diagnostic, `not-subscriptable`, `typing.Union`, the three
command forms, `.venv`, `--python`, `--ty-version`, Python 3.14, and `requires-python`. Conceptual
searches covered target Python/version, interpreter and virtual-environment discovery,
`VIRTUAL_ENV`, `TY_UV`, workspace metadata, and the ty/uv project integration. Open and closed
issues and open, closed, and merged pull requests were included. No exact prior tracker or later fix
was found. The implementation deliberately enables uv metadata discovery for `uv check`; a
maintainer confirmed the resulting precedence is intentional current behavior while noting that
the preferred long-term safety tradeoff remains under discussion.

The implementation and discussion chains for astral-sh/uv#19605, astral-sh/uv#19655,
astral-sh/uv#19763, astral-sh/uv#20501, and astral-sh/uv#20643 were inspected. In addition to
astral-sh/uv#19790, astral-sh/uv#21551 and astral-sh/uv#21211 were plausible `uv check` candidates
but were ruled out: they concern configured file exclusions and ty release cutoff selection,
respectively, not Python target-version inference.
