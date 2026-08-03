# Bad error message with parser error and no-build

Issue: astral-sh/uv#20908
Classification: bug

## Summary

The closest precedent is astral-sh/uv#10522 and its fix astral-sh/uv#10553 for preserving useful pyproject parse causes; astral-sh/uv#10204 is adjacent background for non-package semantics. None tracks the reported `uv lock` metadata fallback involving malformed dependencies, `no-build`, and `package = false`.

## Classification

The report provides a concrete `uv lock` reproduction where malformed dependency syntax is obscured by an unrelated build-disabled message, and where an existing `package = false` setting does not control the fallback behavior as intended. These are correctness and misleading-output problems, not requests for new functionality. No existing issue or pull request was found that already tracks this same combination closely enough for duplicate classification.

## Related

- https://github.com/astral-sh/uv/issues/10522 (closed issue): uv venv "Failed to parse: `pyproject.toml`" warning without the root cause + why the project version in `pyproject.toml`
  astral-sh/uv#10522 concerns the same broad error-reporting failure: a pyproject parsing cause was hidden behind a less useful message. It differs because it involved `uv venv` settings discovery and a missing version, not `uv lock`, invalid dependency syntax, `no-build`, or metadata fallback.
- https://github.com/astral-sh/uv/pull/10553 (merged pull request): Provide `pyproject.toml` path for parse errors in `uv venv`
  astral-sh/uv#10553 fixed astral-sh/uv#10522 by retaining the detailed parse error. It is relevant precedent, but its narrowly scoped `uv venv` change does not cover the reported lock and metadata-generation path.
- https://github.com/astral-sh/uv/issues/10204 (open issue): Add a non-package-mode
  astral-sh/uv#10204 is the canonical broader discussion of non-package projects and documents `[tool.uv] package = false`. The new issue identifies a narrower correctness problem where that existing setting is allegedly ignored during recovery from malformed project metadata, so discussion cannot be fully centralized in the broader design issue.
