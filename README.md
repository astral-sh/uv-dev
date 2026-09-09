# uv check overrides ty exclusions for automatically selected workspace members

Issue: astral-sh/uv#21551

Classification: bug

## Summary

The reported behavior is reproducible. In a virtual workspace whose only member is beneath
`vendor`, `[tool.ty.src].exclude = ["vendor"]` prevents ty's normal source discovery from checking
that member, but plain `uv check` automatically selects the workspace member and reports its type
error. `uv check --no-project` respects normal ty discovery and reports no diagnostics.

The observed subprocess command and current implementation agree on the mechanism: a virtual
workspace with no explicit package selection is treated like `--all-packages`, and uv passes each
member root to ty as a positional path. No existing integration test combines automatic workspace
selection with `[tool.ty.src].exclude`.

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

Those tests do not configure `[tool.ty.src].exclude` or assert its interaction with uv-generated
positional targets.

## Draft response

Thanks for the clear report. I reproduced this on uv 0.12.11 with the same automatically selected
ty 0.0.79. In a minimal virtual workspace with `vendor/vendored` as a member and
`[tool.ty.src].exclude = ["vendor"]`, plain `uv check` passed `vendor/vendored` to ty as a positional
path and reported its type error. `uv check --no-project` passed no positional target and succeeded.

The existing workspace-selection tests cover checking all virtual-workspace members by default,
but not the interaction with ty source exclusions. This should remain open as a focused bug. The
fix will need to account for ty's exclusion-pattern semantics: with ty 0.0.79, `--force-exclude`
did not suppress an explicitly selected member for the bare pattern `vendor`, although
`vendor/**` did.

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
