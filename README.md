# `uv check` and `ty check` report different errors

Issue: astral-sh/uv#21720

Classification: question

## Summary

With `project.requires-python = ">=3.12"` and a Python 3.14 project environment, standalone
`ty check` and `uv run ty check` accept the example, while `uv check` reports a
`not-subscriptable` diagnostic for `typing.Union`. Changing the requirement to Python 3.14 makes
standalone ty report the diagnostic, and selecting Python 3.12 with `uv check --python 3.12` makes
`uv check` pass. Pinning the same ty release through `--ty-version` does not remove the difference.

The difference follows the current design. `requires-python = ">=3.12"` permits the Python 3.14
environment. `uv check` deliberately gives ty uv workspace metadata containing that environment's
concrete interpreter, while standalone ty derives its default target from the lower bound of
`project.requires-python`. No existing issue tracks a request to change this behavior. The nearest
open issue, astral-sh/uv#19790, has the same command contrast but a separate invalid-environment
failure.

## Draft response

`requires-python` is not being ignored here: `>=3.12` permits the Python 3.14 interpreter in
`.venv`. `uv check` synchronizes and checks against its selected project environment, then enables
ty's uv integration, which supplies that interpreter version as the target. Standalone `ty check`
and `uv run ty check` do not enable that integration, so ty derives its default target from the
lower bound of `project.requires-python` instead. `--ty-version` only selects the ty executable, so
it does not affect the target Python version.

To check the Python 3.12 behavior with `uv check`, use `uv check --python 3.12`. If the type-checking
target should remain 3.12 regardless of the selected environment, set
`[tool.ty.environment] python-version = "3.12"`, which has higher precedence than uv-supplied
environment metadata.

## Classification

This is a question because the report primarily asks why the commands differ, and the source
establishes that the distinction is deliberate. `requires-python` is a compatibility range, not an
instruction for `uv check` to ignore its selected project interpreter. Python 3.14 satisfies
`>=3.12`. The current uv source sets `TY_UV=1` for `uv check`, and workspace metadata exposes the
selected environment's concrete interpreter. Current ty source applies that uv-supplied interpreter
version as an environment option. Without uv integration, ty instead derives its default target
from the lower bound of `project.requires-python`.

No incorrect behavior or explicit request to change that precedence is established. This is also
not a duplicate of astral-sh/uv#19790: that issue concerns an invalid local-environment error when
`/usr/local` is configured as a system project environment, whereas astral-sh/uv#21720 uses a valid
`.venv` and successfully checks against its concrete Python version.

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
also define `--ty-version` solely as ty executable selection. Ty's configuration precedence places
project `[tool.ty.environment]` settings above uv workspace metadata, so an explicit
`python-version = "3.12"` provides an environment-independent type-checking target.

Literal searches covered the complete diagnostic, `not-subscriptable`, `typing.Union`, the three
command forms, `.venv`, `--python`, `--ty-version`, Python 3.14, and `requires-python`. Conceptual
searches covered target Python/version, interpreter and virtual-environment discovery,
`VIRTUAL_ENV`, `TY_UV`, workspace metadata, and the ty/uv project integration. Open and closed
issues and open, closed, and merged pull requests were included. No exact prior tracker or later fix
was found because the observed precedence is implemented deliberately rather than being a known
fixed regression.

The implementation and discussion chains for astral-sh/uv#19605, astral-sh/uv#19655,
astral-sh/uv#19763, astral-sh/uv#20501, and astral-sh/uv#20643 were inspected. In addition to
astral-sh/uv#19790, astral-sh/uv#21551 and astral-sh/uv#21211 were plausible `uv check` candidates
but were ruled out: they concern configured file exclusions and ty release cutoff selection,
respectively, not Python target-version inference.
