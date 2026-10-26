# `uv check` and `ty check` report different errors

Issue: astral-sh/uv#21720

Classification: question (original triage)

Status: Maintainer reports the fix shipped in ty 0.0.82.

## Summary

Maintainer zsol confirmed on 2026-10-26 that the fix for astral-sh/uv#21720 shipped in ty 0.0.82
(comment ID 6427122145). The reproduction below covers ty 0.0.81; the fixed release has not been
independently verified in the recorded investigation. The comment does not identify a fixing pull
request or specify which target-version precedence rules changed.

With ty 0.0.81, `project.requires-python = ">=3.12"`, and a Python 3.14 project environment, standalone
`ty check` and `uv run ty check` accept the example, while `uv check` reports a
`not-subscriptable` diagnostic for `typing.Union`. Changing the requirement to Python 3.14 makes
standalone ty report the diagnostic, and selecting Python 3.12 with `uv check --python 3.12` makes
`uv check` pass. Pinning the same ty release through `--ty-version` does not remove the difference.

The reported behavior is reproducible with the reported uv and ty versions on Linux. Verbose ty
output confirms that standalone ty targets Python 3.12, the lower bound of `requires-python`, while
the uv-integrated invocation targets the Python 3.14 project environment. A maintainer confirmed
that preferring the existing environment was intentional at the time, while also noting debate
about whether minimum-version compatibility checking should take precedence. That assessment
predates the confirmation of a released fix. The reporter clarified that the practical impact was a `ty-pre-commit` failure:
the hook runs `uv check`, while a direct `ty check` on the same project passes.

## Reproduction

Outcome: **reproducible with ty 0.0.81**. No recorded retest with ty 0.0.82.

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

## Maintainer assessment before the fix

The maintainer described the preference in ty 0.0.81 as intentional and following the environment-oriented behavior of other uv
commands: `uv check` uses the existing compatible project environment and passes its Python version
to ty. This is useful when dependency selection changes with Python markers, since checking against
the concrete environment keeps type analysis aligned with the packages actually selected there.

The competing goal is minimum-version compatibility. Standalone ty derives its target from the
lower bound of `project.requires-python`, which can catch use of Python or typing features that are
unavailable on versions the project claims to support. The maintainer comment identifies this as a
potentially more important safety property. This explains the earlier design tradeoff; it does not
establish the precedence implemented by ty 0.0.82.

Changing only the Python-version preference would not make `uv check` a drop-in replacement for
`ty check`. The commands also intentionally differ in scope: `uv check` can select multiple
workspace members where `ty check` selects the current project, and `uv check` excludes PEP 723
scripts unless explicitly requested while ty includes them without preparing their environments.

## Reported workflow impact

The reporter is comfortable with the commands having different semantics if the distinction is
clear. Their unexpected failure came from `ty-pre-commit`: the hook invokes `uv check`, so it checks
the concrete project environment and can fail even when a developer's direct `ty check` passes
against the project's minimum declared Python version. They ask whether the hook should invoke ty
directly instead. The later fix announcement does not say that the hook's entry command changed.

## Resolution and verification

The actionable release information is ty 0.0.82, as reported by the maintainer. A follow-up
verification should rerun the existing reproduction with that ty version and confirm which ty
version the pre-commit hook selects. The existing reproduction pins ty 0.0.81, so running it
unchanged would continue to exercise the affected release. The Python 3.12 interpreter workaround
was verified for that older release; no new workaround or hook configuration was supplied with
the fix announcement.

## Classification

The original triage classified this as a question because a maintainer had confirmed that the
existing-environment preference was intentional behavior, and the reporter's follow-up asked
which intentional command semantics `ty-pre-commit` should expose. `requires-python` is a
compatibility range, and Python 3.14 satisfies `>=3.12`. The current uv source sets `TY_UV=1` for
`uv check`, and workspace metadata exposes the selected environment's concrete interpreter.
Observed verbose output confirms that ty uses that Python 3.14 version under uv integration, while
standalone ty uses the lower bound of `project.requires-python`.

The subsequent maintainer report of a fix supersedes the earlier unresolved status. The original
classification is retained as triage history; it should not be read as a claim that no fix was
needed or released. The announcement does not explain whether the change was treated as a bug fix
or a design adjustment. This is also not a duplicate
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
  `uv check`. The ty source inspected during the original investigation consumed that interpreter version as uv-supplied configuration,
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
was found during the original searches. The later maintainer comment identifies ty 0.0.82 as the
fixed release but provides no fixing pull request. Earlier source and reproduction findings below
describe the affected behavior, not a verification of the fixed release.

The implementation and discussion chains for astral-sh/uv#19605, astral-sh/uv#19655,
astral-sh/uv#19763, astral-sh/uv#20501, and astral-sh/uv#20643 were inspected. In addition to
astral-sh/uv#19790, astral-sh/uv#21551 and astral-sh/uv#21211 were plausible `uv check` candidates
but were ruled out: they concern configured file exclusions and ty release cutoff selection,
respectively, not Python target-version inference.
