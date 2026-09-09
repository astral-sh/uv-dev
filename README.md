# uv check overrides ty exclusions for automatically selected workspace members

Issue: astral-sh/uv#21551

Classification: bug

## Summary

`uv check` reports diagnostics from a workspace member beneath a directory configured in
`[tool.ty.src].exclude`. The reported virtual workspace lists `vendor/sglang/python` as a member
while excluding `vendor`; plain `uv check` checks that member, whereas `uv check --no-project`
respects the exclusion.

The current source supports the reported mechanism. In a virtual workspace with no explicit
package selection, `crates/uv/src/commands/project/check.rs` treats the invocation as
`--all-packages`, constructs the check targets from every workspace member root, and passes those
targets to ty. `crates/uv/src/commands/project/check/ty.rs` places the targets after `--` as
positional paths but does not enable ty's force-exclusion behavior. With `--no-project`, uv does not
discover the workspace or generate those positional member targets. Existing integration tests
confirm that a virtual workspace checks all members by default, but do not cover a member matched
by `[tool.ty.src].exclude`.

No earlier issue or pull request was found for this exact interaction. The behavior originates in
the workspace-selection implementation in astral-sh/uv#20628, which completed the request in
astral-sh/uv#20233. That pull request merged before uv 0.12.0, while this report is against uv
0.12.10; no intervening exclusion fix was found. Other related work concerns different kinds of
selection or configuration.

## Draft response

Thanks for the clear report. The current implementation does treat a virtual workspace with no
package selection as `--all-packages` and passes each workspace member root to ty as an explicit
check target. Since uv does not also force configured exclusions for those internally selected
paths, `[tool.ty.src].exclude` can be bypassed in exactly the way you describe; `--no-project`
avoids those generated targets.

The existing workspace-selection work in astral-sh/uv#20233 and astral-sh/uv#20628 does not track
this interaction, so this should remain open as a separate bug. The next step is a focused
regression test with an automatically selected member beneath an excluded directory, followed by a
decision on whether uv should request ty's force-exclusion behavior or filter its generated targets
before invoking ty.

## Classification

This is a `bug`. Source confirms that uv internally turns automatically discovered workspace
members into explicit ty paths. The user did not explicitly select those paths, yet that internal
translation changes the meaning of their ty exclusion configuration. The difference between plain
`uv check` and `uv check --no-project` follows directly from whether uv discovers the workspace and
generates member targets.

This is not a duplicate of the earlier package-selection request or custom-config-file request.
astral-sh/uv#20233 asked for selecting workspace packages, and astral-sh/uv#19791 asks for choosing a
different ty configuration file; neither tracks exclusions being overridden by uv-generated
positional paths. It is also not a regression of a previously fixed exclusion bug: no earlier
matching report or fix was found.

## Related

- astral-sh/uv#20628 (merged pull request), "Add `--package` and `--all-packages` to `uv check`" —
  the direct implementation origin. It made a virtual workspace with no package selection
  equivalent to `--all-packages` and added explicit member-root check targets so ty would follow
  uv's workspace selection. It shipped by uv 0.12.0 and did not cover
  `[tool.ty.src].exclude` precedence.
- astral-sh/uv#20233 (closed issue), "Support for --package and --all-packages in `uv check`" — the
  request completed by astral-sh/uv#20628. It concerns which workspace packages uv selects, not
  whether ty exclusions remain effective for automatically generated targets.
- astral-sh/uv#20676 (merged pull request), "Avoid checking any scripts in `uv check` unless
  `--script` is passed" — adjacent selection/exclusion precedent. uv deliberately excludes
  automatically discovered PEP 723 scripts, but this change does not address ty source exclusions
  or excluded workspace members.
- astral-sh/uv#19791 (open issue), "Allow users to specify a custom ty configuration file when
  running `uv check`" — related configuration-forwarding work, but a different config file would
  not stop uv-generated positional member paths from taking precedence over its exclusions.

## Search and evidence scope

Literal issue and pull-request searches covered `force-exclude`, `tool.ty.src`, `src.exclude`,
`uv check --no-project`, explicit paths, exclusions, and vendored workspace members. Conceptual
searches covered workspace/package selection, type-check ownership, selection semantics, ty
configuration, positional-path precedence, and automatic member discovery. Fix-oriented searches
covered closed issues and merged changes around the introduction of workspace selection, including
astral-sh/uv#20628, astral-sh/uv#20649, and astral-sh/uv#20676, and their comments and references.

astral-sh/uv#20649 was ruled out because it only repaired workspace snapshots and Ruff lints after
astral-sh/uv#20628. astral-sh/uv#21083 was ruled out because it concerns avoiding installation of a
project with native extensions, not controlling ty's checked paths. The reporter-suggested
astral-sh/uv#20233 and astral-sh/uv#19791 are related but do not track the same behavior.
