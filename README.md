# uv tree --group dev not showing only dev

Issue: astral-sh/uv#21064

Classification: question

## Summary

The reporter is using uv 0.12.2 on Linux with Python 3.14. Their partial `pyproject.toml`
defines `dev` and `airflow` dependency groups. They report that
`uv tree --group dev --depth 1` displays dependencies from both groups and expect it to display
only `dev`.

The command uses the additive group-selection flag. Repository help text describes `--group` as
including a named group, while `--only-group` includes only the named groups, omits the project and
its regular dependencies, and implies `--no-default-groups`. The nearby `uv tree` integration test
also shows that `--group foo` retains the default `dev` group, whereas `--only-group bar` restricts
the group selection to `bar`.

The provided configuration is incomplete, so it does not establish why `airflow` is selected in
this specific project. In the absence of a `tool.uv.default-groups` setting, uv's project default is
the `dev` group; the omitted project or workspace configuration is therefore needed only if
`airflow` still appears with the restrictive command.

## Draft response

`--group` is additive: it includes the named group alongside the project's default groups. To
display only `dev`, use `uv tree --only-group dev --depth 1`; `--only-group` also disables the
default groups.

If `airflow` still appears with that command, please provide the complete `pyproject.toml`
(including `[tool.uv]` and any workspace configuration) and the full tree output so we can
reproduce it.

## Classification

Classify this as a `question`. The source, CLI help, documentation, and integration snapshots all
establish that `--group` is intentionally additive and that `--only-group` is the existing
restrictive interface. The report does not show `--only-group` behaving incorrectly, and the
partial `pyproject.toml` is insufficient to establish that a non-default group is selected despite
the configured defaults. If a complete reproduction shows that `--only-group dev` also includes
`airflow`, that would instead establish a bug.

This is not a duplicate of the closest open discussion. astral-sh/uv#19973 questions the overall
asymmetry of `uv tree`'s group and extra flags, but it explicitly recognizes the current standard
default-group semantics and does not report failure of the restrictive option.

## Related

- astral-sh/uv#19973 (open issue), “`uv tree`'s flags for extra/groups are weird”: the closest
  ongoing design discussion. It confirms that `uv tree` currently uses standard additive and
  default-group semantics while questioning the broader extras/groups asymmetry. It does not track
  a failure of `--only-group`.
- astral-sh/uv#8338 (merged pull request), “Add `--group`, `--only-group`, and `--only-dev` support
  to `uv tree`”: introduced both the additive `--group` option and the restrictive `--only-group`
  option, directly establishing the intended distinction.
- astral-sh/uv#12526 (closed issue), “uv tree with --only-group doesn't show full depth”: a
  historical correctness bug in the restrictive mode. It concerned missing transitive dependencies
  and was fixed by astral-sh/uv#12560; it differs from this report, which invokes `--group`.

## Search and supporting evidence

Literal searches covered `uv tree --group`, `tree --group`, `tree group dev`, `--group` with the
reported group names, and dependency-group terminology. Conceptual searches covered group-only
selection, filtering, default groups, selected groups, all groups, and dependency-tree output.
Searches included open and closed issues and open, closed, and merged pull requests. Fix-oriented
inspection covered astral-sh/uv#8338, astral-sh/uv#12526, astral-sh/uv#12560,
astral-sh/uv#10890, and astral-sh/uv#11224.

astral-sh/uv#19976 was inspected and ruled out because it concerns inconsistent `--depth` behavior
for scripts and workspace-group roots, not group selection. astral-sh/uv#19327 was also ruled out:
it concerned optional-extra edges leaking between shared package nodes and was fixed by
astral-sh/uv#19332. astral-sh/uv#19975 concerns dependency-group-only projects failing without a
`project` table, so it does not match this report either.
