# uv sync produces incorrect warning about entry points (project.scripts) in a child package within a workspace

Issue: astral-sh/uv#22111

Classification: bug

## Summary

The reporter's root project depends on a child workspace member through a
`workspace = true` source. The member declares `project.scripts`, and `uv sync`
installs the script into `.venv/bin`, but also warns that entry points are being
skipped because the member is not packaged.

The exact child configuration shown uses `[tools.uv]`, while the supported table
is `[tool.uv]`. Consequently, the shown `package = true` is not read. Correcting
that spelling should suppress the warning. However, this is still a real
false-positive for the configuration as posted: uv records a member referenced
by a workspace source as a required member and treats it as installable even
without an explicit package setting or build system. The warning path instead
calls `is_package(true)` unconditionally, so it classifies the same member as
non-packaged and claims its entry points were skipped.

The warning was extended to workspace members by astral-sh/uv#18389, which
resolved astral-sh/uv#18388. That test covers a non-required member selected via
`--all-packages`, where the warning is correct, but not a member required by the
root project. astral-sh/uv#14891 independently confirms that making a workspace
member a root dependency is what causes that member's entry points to be
installed.

## Draft response

Thanks for the report. One detail in the child snippet is that the table is
written as `[tools.uv]`; the supported table is `[tool.uv]`. If the plural form
is present in the actual file, `package = true` is not being applied, and
correcting the table name should remove this warning.

There is still a warning bug in the exact setup shown. Because the root project
depends on `ccaas-data-tables` through a workspace source, uv treats that member
as required and installs it, which is why the script appears in `.venv/bin`.
The warning check does not account for that required-member behavior and can
incorrectly say the entry points were skipped. We should align the warning with
the installation decision and cover this case with a regression test.

If `[tools.uv]` was only a typo in the issue and the actual file already uses
`[tool.uv]`, could you confirm that? That would indicate a different path,
since an explicit `tool.uv.package = true` should make the current warning check
false.

## Classification

This is a bug because the user-facing warning contradicts the operation uv
actually performs. The mechanism is source-confirmed: workspace source entries
populate `required_members`; installation evaluates such a member with
`is_package(false)`, while the warning introduced for workspace members always
evaluates it with `is_package(true)`. With no recognized explicit package
setting, those calls return different results.

The plural `[tools.uv]` table explains why the explicit setting shown is ignored,
but it does not make the warning accurate: the root dependency still makes the
member installable and its entry point is not skipped. This is not a duplicate.
The closest prior issue and pull request cover the opposite missing-warning case,
not this false positive, and no open issue or pull request was found that already
tracks the mismatch.

## Related

- astral-sh/uv#18388 (closed), "No warning printed about entrypoint installation
  being skipped for workspace members" — the direct historical counterpart. It
  established that non-root members should emit this warning when scripts are
  genuinely skipped because the member is not packaged.
- astral-sh/uv#18389 (merged), "Warn when workspace member scripts are skipped
  due to missing build system" — resolved astral-sh/uv#18388 and introduced the
  current loop over selected workspace members. Its warning predicate uses
  `is_package(true)` and its new test does not make the member a dependency of
  the root, so it does not cover required-member installation.
- astral-sh/uv#14891 (closed), "Install entry points for workspace members" — an
  adjacent report whose resolution was to add the member as a dependency of the
  workspace root. It confirms the important distinction that required members'
  entry points are installed, but it did not report the contradictory warning.

## Search and supporting evidence

Searches covered the full warning text and reduced fragments (`project.scripts`,
`not packaged`, and `Skipping installation of entry points`), the exact
`tool.uv.package`/`tools.uv` identifiers, `uv sync`, workspace child/member and
root-dependency terminology, required/virtual package behavior, and historical
fixes across open and closed issues plus open, closed, and merged pull requests.
The strongest chain was astral-sh/uv#18388 to astral-sh/uv#18389, followed by
astral-sh/uv#14891 for required-member behavior.

astral-sh/uv#7428 was inspected because the warning implementation links to it,
but it requests richer parsing and more selective warning content rather than
tracking this false-positive predicate. astral-sh/uv#11583 was also inspected
and ruled out: it concerns an unreproduced import failure from a built wheel,
not a warning emitted while entry points are successfully installed.
