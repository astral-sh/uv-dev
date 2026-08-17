# author field in pyproject.toml is being interpret as TOML 1.1 by uv build

Issue: astral-sh/uv#21065

Classification: enhancement

## Summary

The reporter uses `uv build --sdist` with uv 0.12.0 and finds both `pyproject.toml` and
`pyproject.toml.orig` in the archive. They infer that uv classified this TOML 1.0-compatible value
as TOML 1.1 syntax:

```toml
[project]
author = [{ name = "rzuckerm" }]
```

Repository evidence shows that the field did not trigger TOML-version detection. Starting in uv
0.12, uv_build unconditionally parses and serializes the source distribution's root
`pyproject.toml` into TOML 1.0-compatible syntax and preserves the source file as
`pyproject.toml.orig`. astral-sh/uv#18741 introduced the compatibility rewrite, and
astral-sh/uv#20225 deliberately removed the TOML 1.1 detector and made the rewrite-and-preserve
path unconditional when stabilizing the feature for uv 0.12.

The shown value is valid TOML 1.0 syntax. Separately, PEP 621 defines the standardized project
metadata key as `project.authors` (plural), not `project.author`; that semantic naming issue does
not explain the `.orig` file.

## Reported impact and workaround

The reporter later clarified that the additional file affected their downstream build system. When
Renovate changed its build requirement from `uv_build>=0.11,<0.12` to
`uv_build>=0.12,<0.13`, that system treated `pyproject.toml.orig` as a newly added project file and
produced multiple unnecessary releases. This impact is reporter-supplied and has not been
independently reproduced here. The reporter also says a newer version of their build system now
handles the file, providing a downstream workaround for future updates.

## Draft response

The presence of `pyproject.toml.orig` does not mean this field was interpreted as TOML 1.1. In uv
0.12, uv_build unconditionally rewrites the source distribution's root `pyproject.toml` to TOML
1.0-compatible syntax and preserves the input as `pyproject.toml.orig`. That behavior was
introduced in astral-sh/uv#18741 and made the default in astral-sh/uv#20225.

The shown value is valid TOML 1.0 syntax, although the standardized PEP 621 metadata key is
`project.authors` (plural). Omitting `.orig` for already-compatible inputs would be a change to the
stabilized behavior rather than a parser fix.

## Classification

This is an enhancement rather than a bug or duplicate. The reported extra file is intentional,
documented uv 0.12 behavior: astral-sh/uv#20225 explicitly removed syntax detection, always
normalizes the sdist's root `pyproject.toml`, and always retains `pyproject.toml.orig`. The review of
astral-sh/uv#18741 considered writing `.orig` only when necessary, but the maintainer chose
consistent normalization and an always-available original. Therefore omitting the backup for TOML
1.0-compatible inputs would change established behavior rather than correct a parser defect. No
open or closed issue or pull request was found that tracks that requested change closely enough to
centralize discussion there.

## Related

- astral-sh/uv#20225 — “Stabilize preview features for uv 0.12” (merged pull request). This is the
  closest version-specific match: its body and diff say that uv 0.12 always rewrites the sdist
  `pyproject.toml`, preserves `pyproject.toml.orig`, and removes the earlier TOML 1.1 detector.
- astral-sh/uv#18741 — “Add TOML v1.1 -> v1.0 backwards compatibility for source distributions”
  (merged pull request). This introduced the transformation. Its review directly asks whether
  `.orig` should be written only when rewriting is necessary; the maintainer explains the choice to
  normalize consistently and make the original consistently available.
- astral-sh/uv#20185 — “uv 0.12 preview stabilization tracking issue” (closed issue). This lists
  `toml-backwards-compatibility` among the features deliberately stabilized for the exact reported
  release and was closed by astral-sh/uv#20225.
- astral-sh/uv#20049 — “Warn when `pyproject.toml` uses TOML 1.1 syntax (e.g. multi-line inline
  tables) that Python's `tomllib` can't parse” (open issue). Maintainer comments confirm that
  uv_build rewrites source distributions. It is adjacent rather than canonical because it requests
  diagnostics for actual TOML 1.1 syntax used with other build backends.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
used `pyproject.toml.orig`, the shown `author`/`authors` form, TOML 1.1, and `uv build --sdist`.
Conceptual and fix-oriented queries covered source-distribution normalization, TOML downgrade and
backwards compatibility, disabling the rewrite, omitting the backup, and uv 0.12 stabilization.
The strongest candidates' bodies, diffs, reviews, comments, references, and relevant current source
were inspected.

No existing issue or pull request requests an opt-out or omission of `.orig` for TOML 1.0 inputs.
astral-sh/uv#18055 was ruled out because `pyproject.toml.orig` is merely the name of a local diff
input there and its packaging problem is unrelated. astral-sh/uv#19832 and astral-sh/uv#21061 were
also inspected and ruled out as unrelated despite a misleading association in the checkout's local
history.
