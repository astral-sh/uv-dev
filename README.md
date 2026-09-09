# uv check overrides ty exclusions for automatically selected workspace members

Issue: astral-sh/uv#21551

Classification: bug

## Summary

The reported behavior is reproducible. In a virtual workspace whose only member is beneath
`vendor`, `[tool.ty.src].exclude = ["vendor"]` prevents ty's normal source discovery from checking
that member, but plain `uv check` automatically selects the workspace member and reports its type
error. `uv check --no-project` respects normal ty discovery and reports no diagnostics.

The observed subprocess command and original implementation agree on the mechanism: a virtual
workspace with no explicit package selection was treated like `--all-packages`, and uv passed each
member root to ty as a positional path. The parent regression extended the existing virtual
workspace integration test to cover this interaction.

## Reproduction

Outcome: **reproducible**.

Environment:

- uv 0.12.11 (`x86_64-unknown-linux-gnu`); the report used uv 0.12.10 on the same target
- ty 0.0.79, automatically selected by uv (the same version reported)
- CPython 3.12 at `/usr/bin/python3.12`
- Linux 6.17.0-1022-azure x86_64
- All fixture files, the uv cache, and the uv-managed Python directory were isolated under `/tmp`

Minimal fixture:

```toml
# pyproject.toml
[tool.uv.workspace]
members = ["vendor/vendored"]

[tool.ty.src]
exclude = ["vendor"]
```

```toml
# vendor/vendored/pyproject.toml
[project]
name = "vendored"
version = "0.1.0"
requires-python = ">=3.12"
```

```python
# vendor/vendored/bad.py
value: int = "not an integer"
```

With `UV_CACHE_DIR` and `UV_PYTHON_INSTALL_DIR` pointed inside the temporary fixture and inherited
`UV_LOCKED` disabled, run:

```console
$ uv check --isolated --python /usr/bin/python3.12 --color never --show-version --show-command
Using ty 0.0.79
Running `ty check --color never --no-progress --exclude-scripts -- vendor/vendored`
error[invalid-assignment]: Object of type `Literal["not an integer"]` is not assignable to `int`
 --> vendor/vendored/bad.py:1:14
Found 1 diagnostic
```

The command exits 1. The control command succeeds:

```console
$ uv check --isolated --no-project --ty-version 0.0.79 --color never --show-version --show-command
Using ty 0.0.79
Running `ty check --color never --no-progress --exclude-scripts`
All checks passed!
WARN No python files found under the given path(s)
```

Direct ty checks confirm that the positional member target is the relevant behavioral difference:
`ty check` discovers no Python files, while `ty check -- vendor/vendored` reports the same error.
One nuance for the proposed remedy is that ty 0.0.79's `--force-exclude` still reports the file with
the exact bare pattern `vendor`; an explicit recursive pattern such as `vendor/**` is excluded.
Therefore the reproduction confirms the report, but does not establish that adding
`--force-exclude` alone would handle every currently valid exclusion spelling.

Nearby integration coverage is in `crates/uv/tests/project/check.rs`:

- `check_virtual_workspace_checks_all_members_by_default` verifies that plain `uv check` at a
  virtual-workspace root checks every member.
- `check_workspace_member_selection` verifies implicit and explicit selection of one member.
- `check_workspace_member_inherits_workspace_configuration` verifies that a selected member uses
  workspace-level ty rule configuration.

Before the parent regression, those tests did not configure `[tool.ty.src].exclude`. The updated
virtual-workspace test now covers its interaction with uv-generated member selection.

## Fix

Outcome: **fixed**.

For the default selection of all members in a virtual workspace, uv now passes the in-workspace
member roots to ty through a `src.include` configuration override instead of positional path
arguments. ty applies configured `src.exclude` patterns to configuration includes with their normal
precedence, so the vendored member is omitted while the non-excluded member is still checked.
Explicit `--package`, `--all-packages`, and script selections retain their existing positional-path
behavior. If an automatically selected target cannot be represented as an in-workspace UTF-8
include, uv preserves the prior positional fallback.

The parent regression in `crates/uv/tests/project/check.rs`,
`check_virtual_workspace_checks_all_members_by_default`, now expects only the diagnostic from the
non-excluded member and snapshots the generated ty command with
`src.include = ["packages/member-a", "vendor/vendored"]`. The following focused debug-profile
validation passed:

- `cargo test --package uv --test project check::check_virtual_workspace` (4 tests)
- `cargo test --package uv --test project check::check_workspace` (7 tests)
- `cargo +stable clippy --package uv --test project -- -D warnings`
- `cargo +stable fmt --all -- --check`

The repository's pinned toolchain lacked writable access to install its missing rustfmt and clippy
components, so the already-installed stable toolchain of the same Rust version was used for those
two checks.

## Draft response

Thanks for the clear report. I reproduced this on uv 0.12.11 with the same automatically selected
ty 0.0.79. The fix now represents automatically selected virtual-workspace members as ty source
includes rather than explicit positional paths. This preserves `[tool.ty.src].exclude`, so the
vendored member is skipped while other workspace members are still checked. The focused regression
also snapshots the generated ty command, and neighboring workspace-selection tests continue to
pass unchanged.

## Classification

This is a `bug`. The behavior is directly observed, not inferred solely from source: uv's displayed
ty command contains the automatically generated positional path, the excluded file is diagnosed,
and removing project discovery via `--no-project` makes the check pass. The user did not explicitly
select the member path, so uv's automatic workspace selection changes how their ty exclusion is
applied.

This is not a duplicate of the earlier package-selection or custom-config-file requests.
astral-sh/uv#20233 concerns selecting workspace packages, and astral-sh/uv#19791 concerns choosing a
different ty configuration file; neither tracks exclusions overridden by uv-generated paths.

## Related

- astral-sh/uv#20628 (merged pull request), "Add `--package` and `--all-packages` to `uv check`" —
  introduced the workspace-selection behavior and made a virtual workspace with no package
  selection equivalent to `--all-packages`.
- astral-sh/uv#20233 (closed issue), "Support for --package and --all-packages in `uv check`" — the
  selection request completed by astral-sh/uv#20628.
- astral-sh/uv#20676 (merged pull request), "Avoid checking any scripts in `uv check` unless
  `--script` is passed" — adjacent selection/exclusion behavior, but it does not address ty source
  exclusions for workspace members.
- astral-sh/uv#19791 (open issue), "Allow users to specify a custom ty configuration file when
  running `uv check`" — related configuration forwarding, but not the same exclusion-precedence
  interaction.

Pull request: https://github.com/astral-sh/uv-dev/pull/991
