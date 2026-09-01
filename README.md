# support `uv lint` command to run `ruff check`

Issue: astral-sh/uv#21392

Classification: duplicate

## Summary

The reporter requests a new `uv lint` command that delegates to `ruff check`, plus a `uv lint --fix`
form corresponding to `ruff check --fix`. The motivation is command symmetry with the existing
`uv format` integration for Ruff formatting and `uv check` integration for ty type checking.

The same dedicated Ruff-check shorthand is already requested by astral-sh/uv#16314. A second open
issue, astral-sh/uv#19768, discusses the broader interface choice of adding Ruff and formatting checks
to `uv check` or exposing Ruff through a separate `uv lint` command, including fix behavior. No
related open, closed-unmerged, or merged pull request was found.

## Draft response

Thanks. This is already tracked in astral-sh/uv#16314, which requests a Ruff check/lint counterpart
to `uv format`. astral-sh/uv#19768 also discusses whether Ruff linting should be exposed through
`uv check` or a separate `uv lint` command, including fix behavior. Let’s centralize the dedicated
`uv lint` request in astral-sh/uv#16314.

## Classification

This is a duplicate of astral-sh/uv#16314. Both issues request a uv-level counterpart to `uv format`
that invokes Ruff's checking/linting operation. The new report makes the proposed spelling
(`uv lint`) and `--fix` forwarding explicit, but those details belong in the existing feature
discussion rather than requiring a separate tracker.

Absent the prior issue, this would be an enhancement: it asks for a new command and does not report
incorrect existing behavior. Duplicate takes precedence because the earlier open issue covers the
same underlying capability.

## Related

- astral-sh/uv#16314 — Open enhancement and the closest match. It predates this report and requests
  the same `ruff check` equivalent to the existing `uv format` shorthand. Its framing is broad enough
  to contain discussion of the proposed `uv lint` name and `--fix` behavior.
- astral-sh/uv#19768 — Open enhancement with substantial overlap. It explicitly proposes either
  incorporating `ruff check` into `uv check` or adding `uv lint`, and also describes fix forwarding.
  It differs by primarily exploring a combined type, lint, and format check command.

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
