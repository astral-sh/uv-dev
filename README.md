# support `uv lint` command to run `ruff check`

Issue: astral-sh/uv#21392

Classification: duplicate

## Summary

The reporter requests a new `uv lint` command that delegates to `ruff check`, plus a `uv lint --fix`
form corresponding to `ruff check --fix`. The motivation is command symmetry with the existing
`uv format` integration for Ruff formatting and `uv check` integration for ty type checking.

The Ruff-check integration is already requested by astral-sh/uv#16314. A repository member closed
astral-sh/uv#21392 as its duplicate and expressed a tentative preference for extending `uv check` to
run both `ruff check` and `ty check` and combine their results, rather than introducing a separate
`uv lint` command. The interface is explicitly not settled yet. A second open issue,
astral-sh/uv#19768, discusses that combined-check design and the alternative `uv lint` interface,
including fix behavior. No related open, closed-unmerged, or merged pull request was found.

## Maintainer decision

A repository member closed astral-sh/uv#21392 as a duplicate of astral-sh/uv#16314. They would
currently prefer `uv check` to run both Ruff and ty and combine the results, but cautioned that this
design is not set in stone.

## Classification

This is a confirmed duplicate of astral-sh/uv#16314. Both issues request a uv-level counterpart to
`uv format` that invokes Ruff's checking/linting operation. The new report makes the proposed
spelling (`uv lint`) and `--fix` forwarding explicit, but those details belong in the existing
feature discussion rather than requiring a separate tracker. The duplicate closure confirms the
canonical issue; it does not establish the final command-line design.

Absent the prior issue, this would be an enhancement: it asks for a new command and does not report
incorrect existing behavior. Duplicate takes precedence because the earlier open issue covers the
same underlying capability.

## Related

- astral-sh/uv#16314 — Open enhancement and confirmed canonical issue. It predates this report and
  requests a `ruff check` equivalent to the existing `uv format` shorthand. A repository member
  closed astral-sh/uv#21392 as its duplicate; whether the capability becomes part of `uv check` or a
  separate command remains undecided.
- astral-sh/uv#19768 — Open enhancement with substantial overlap. It explicitly proposes either
  incorporating `ruff check` into `uv check` or adding `uv lint`, and also describes fix forwarding.
  Its combined-check proposal aligns with the repository member's stated preference, though no
  final design decision has been made.

## Search evidence

Searches covered open and closed issues and open, closed-unmerged, and merged pull requests. Literal
queries included `uv lint`, `ruff check`, `ruff check --fix`, `uv check` with Ruff, and `uv format`.
Conceptual queries included lint/linter commands, Ruff linting and integration, static analysis, code
quality checks, format commands, and task aliases. No related pull request was found.

astral-sh/uv#6308 was inspected and ruled out: despite using the terms format and lint, it requests
formatting or validation of `pyproject.toml`, not running Ruff against Python code. astral-sh/uv#5903
was also ruled out because it tracks general project-defined task aliases rather than a built-in Ruff
integration. Results about failures of `uvx ruff check` were operational execution problems, not this
requested shorthand.
