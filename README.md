# uv tree --group dev not showing only dev

Issue: astral-sh/uv#21064

Classification: question

## Summary

The reporter uses uv 0.12.2 on Linux with Python 3.14. Their partial `pyproject.toml`
defines `dev` and `airflow` dependency groups, and they report that
`uv tree --group dev --depth 1` displays both groups instead of only `dev`.

This behavior is reproducible when `airflow` is a configured default group, but it is the intended
additive behavior of `--group`, not evidence that the option is ignored. `--group dev` adds `dev` to
the project's default groups. `--only-group dev` is the restrictive form and implies
`--no-default-groups`.

The issue omits the complete `[tool.uv]` configuration, so it does not establish whether the
reporter's project explicitly makes `airflow` a default group. With only the dependency-group
definitions shown in the report, uv defaults to `dev` and does not display `airflow`.

## Reproduction

Outcome: `reproducible` as additive default-group selection.

The reproduction used the installed `uv 0.12.3 (x86_64-unknown-linux-gnu)` on Linux x86_64. The
available interpreter was CPython 3.12.3, and `--python-version 3.14` was supplied to use the
reporter's Python version for tree filtering. All project files, the lockfile, and the uv cache were
kept under `/tmp`.

Minimal `pyproject.toml`:

```toml
[project]
name = "group-tree-reproduction"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []

[dependency-groups]
dev = ["typing-extensions==4.15.0"]
airflow = ["iniconfig==2.1.0"]

[tool.uv]
default-groups = ["airflow"]
```

The reported command reproduced both selected groups:

```console
$ uv tree --group dev --depth 1 --python-version 3.14
group-tree-reproduction v0.1.0
├── iniconfig v2.1.0 (group: airflow)
└── typing-extensions v4.15.0 (group: dev)
```

Both restrictive forms displayed only `dev`:

```console
$ uv tree --only-group dev --depth 1 --python-version 3.14
group-tree-reproduction v0.1.0
└── typing-extensions v4.15.0 (group: dev)

$ uv tree --no-default-groups --group dev --depth 1 --python-version 3.14
group-tree-reproduction v0.1.0
└── typing-extensions v4.15.0 (group: dev)
```

After removing `[tool.uv].default-groups`, the original `--group dev` command also displayed only
`dev`; the mere presence of an `airflow` dependency group does not select it.

Existing integration coverage is in `crates/uv/tests/project/tree.rs`, test `group`. Its fixture
defines the implicit default `dev` group plus non-default `foo` and `bar` groups. Its snapshots show
that bare `uv tree` includes `dev`, `uv tree --group foo` includes both `dev` and `foo`, and
`uv tree --only-group bar` includes only `bar`. The test therefore directly covers the additive
versus restrictive distinction, although it does not use a custom `default-groups` value.

## Draft response

`--group` is additive: it includes the named group alongside the project's default groups. To show
only `dev`, use `uv tree --only-group dev --depth 1`. Equivalently, use
`uv tree --no-default-groups --group dev --depth 1` if retaining the project dependencies is useful.

If `airflow` still appears with `--only-group dev`, please provide the complete `pyproject.toml`
(including `[tool.uv]` and workspace configuration) and the full tree output.

## Classification

Classify this as a `question`. The CLI help and the observed behavior establish that `--group` is
additive, while `--only-group` is the existing restrictive interface. The reproduction explains
how both groups can appear when `airflow` is a default group; it does not show a failure of group
selection. If the reporter can show that `--only-group dev` also includes `airflow`, that would be a
different result requiring the complete project or workspace configuration.

This is not a duplicate of astral-sh/uv#19973. That issue questions the overall asymmetry of
`uv tree` group and extra flags, but it recognizes the current default-group semantics rather than
reporting a failure of the restrictive option.

## Related

- astral-sh/uv#19973 (open issue), “`uv tree`'s flags for extra/groups are weird”: the closest
  ongoing design discussion. It confirms the current additive and default-group semantics while
  questioning the broader interface.
- astral-sh/uv#8338 (merged pull request), “Add `--group`, `--only-group`, and `--only-dev` support
  to `uv tree`”: introduced the additive `--group` option and restrictive `--only-group` option.
- astral-sh/uv#12526 (closed issue), “uv tree with --only-group doesn't show full depth”: a
  historical bug involving missing transitive dependencies in restrictive mode, fixed by
  astral-sh/uv#12560. It differs from this report, which invokes `--group`.

## Search and supporting evidence

The implementation resolves dependency-group arguments and then adds the project's default groups
for `uv tree`. The command help describes `--group` as “Include dependencies from the specified
dependency group” and `--only-group` as “Only include dependencies from the specified dependency
group.” The long help further states that `--only-group` implies `--no-default-groups`.

Related searches also ruled out astral-sh/uv#19976 (depth behavior for scripts and workspace-group
roots), astral-sh/uv#19327 and its fix astral-sh/uv#19332 (optional-extra edges shared between package
nodes), and astral-sh/uv#19975 (dependency-group-only projects without a `project` table).
