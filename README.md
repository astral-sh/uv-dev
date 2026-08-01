# `uv sync --only-group lint-format` is still building & installing local project

Issue: astral-sh/uv#20877
Classification: question

## Summary

The closest discussions establish that `--only-group` normally omits the project, while the new report has a distinct self-dependency path through `flake8-carrot`. No same-problem tracker or regression fix was found.

## Classification

The reported behavior is source-confirmed as intentional dependency closure rather than failure to omit the project root. In the reporter's current lockfile, `lint-format` includes `flake8-carrot`, and `flake8-carrot` depends on `typed-classproperties`; uv satisfies that selected dependency with the local editable project. Thus `--only-group` omits the project and its ordinary dependencies by default, but does not discard the project when it is transitively required by the selected group. No existing issue was found tracking the same misunderstanding closely enough for duplicate classification.

## Related

- https://github.com/astral-sh/uv/issues/15215 (open issue): In Docker, installing app code with CLI entrypoint and `--only-group` fails with `uv sync`, works with `uv pip install`
  astral-sh/uv#15215 confirms the intended baseline: `--only-group` omits the project as a root. The new case differs because `lint-format` selects `flake8-carrot`, whose locked dependencies include `typed-classproperties`; uv therefore installs the local project transitively to satisfy that selected dependency.
- https://github.com/astral-sh/uv/issues/16396 (closed issue): uv sync includes packages from excluded dependency group when using `--no-group`/`--only-group`
  astral-sh/uv#16396 is an adjacent report about unexpected builds under group filtering. Maintainers distinguished excluded packages being processed during locking from packages actually selected for installation. Here the lockfile directly shows the selected `lint-format` dependency chain returning to the local project.
