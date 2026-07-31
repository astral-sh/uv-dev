# uvx should not use the project's dependencies

Issue: astral-sh/uv#20871
Classification: question

## Summary

No duplicate or fixing pull request was found. astral-sh/uv#7186 and astral-sh/uv#6376 establish that project dependency reuse is proposed rather than current intended behavior, while astral-sh/uv#8965 documents the adjacent current-directory import-shadowing behavior that may explain this reproduction.

## Classification

Repository documentation, source help, and maintainer comments establish that `uvx` is intended to use an isolated tool environment, including when invoked inside a project. The reproduction uses the same importable package name in the current directory as the published tool, so current-directory import shadowing is a plausible alternative explanation, supported by astral-sh/uv#8965, but it is not yet confirmed for this report. Consequently, no uv correctness defect is established yet; the issue primarily needs clarification of which files are imported and whether the behavior persists when invoked from an unrelated project with a different package name.

## Related

- https://github.com/astral-sh/uv/issues/7186 (open issue): Should `uvx` use a project dependency if available?
  astral-sh/uv#7186 tracks the inverse capability: making `uvx` use project dependencies. Maintainer discussion confirms that `uvx` currently runs tools separately from the project and treats project integration as an undecided enhancement.
- https://github.com/astral-sh/uv/issues/6376 (closed issue): importlib.metadata.distributions() doesn't surface all installed packages/endpoints, breaking pytest plugins
  In astral-sh/uv#6376, maintainers explicitly confirmed that `uvx` uses an environment isolated from the project and recommended `uv run` when project dependencies are required. This establishes the intended semantics, though its symptom is missing rather than unexpectedly visible project dependencies.
- https://github.com/astral-sh/uv/issues/8965 (closed issue): UV Environment Management Tool Confuses Package Paths Across Environments
  astral-sh/uv#8965 contains a closely relevant explanation that Python can import a same-named package directly from the current working directory, independently of what is installed in the selected environment. That could explain this reproduction because the local project and published tool share a module name, but the mechanism has not been confirmed for astral-sh/uv#20871.
