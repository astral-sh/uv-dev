# uv sync produces incorrect warning about entry points (project.scripts) in a child package within a workspace

Issue: astral-sh/uv#22111

Classification: bug

## Summary

The reported warning is reproducible when `uv sync` is run from the child
workspace member using the configuration exactly as posted. In a fresh
environment, uv warns that the member's entry points are being skipped, then
builds and installs that member, creates `.venv/bin/my_script`, and the script
runs successfully.

The child configuration shown in the report uses `[tools.uv]` (plural), while
the supported table is `[tool.uv]`. Changing only that spelling suppresses the
warning. Running `uv sync` from the workspace root also does not warn, even with
the plural spelling, so the working directory or selected package is an
important part of the reproduction.

## Classification

This is a bug for the posted configuration and child-member invocation because
the warning says that entry points are skipped while the same sync installs the
entry point. The misspelled `[tools.uv]` table explains why the explicit
`package = true` setting is ignored, but it does not make the warning's claim
about the observed installation accurate.

If `[tools.uv]` is only a transcription error and the reporter's actual file
uses `[tool.uv]`, the behavior is not reproduced by the supplied fixture and
the actual child `pyproject.toml`, invocation directory, and complete command
would be needed.

## Reproduction

Reproduced on Linux x86_64 with both the reported uv 0.12.3 release and the
installed uv 0.12.13, using managed CPython 3.11.16. The reporter used macOS
15.7.9 x86_64 and Python 3.11.13, so the behavior is not platform-specific in
this fixture.

The minimal workspace uses the root metadata from the report:

```toml
[project]
name = "my-project"
version = "0.1.0"
requires-python = "~=3.11.0"
dependencies = ["ccaas-data-tables"]

[tool.uv.workspace]
members = ["shared/*"]

[tool.uv.sources]
ccaas-data-tables = { workspace = true }
```

The child uses the reported plural table spelling:

```toml
[project]
name = "ccaas-data-tables"
version = "0.1.0"
dependencies = ["click~=8.5.0"]

[tools.uv]
package = true

[project.scripts]
my_script = "my_script:cli"
```

With `my_script.py` defining a `cli` function, run from the child directory:

```console
$ cd shared/ccaas-data-tables
$ uv sync
warning: Skipping installation of entry points (`project.scripts`) for package `ccaas-data-tables` because this project is not packaged; to install entry points, set `tool.uv.package = true` or define a `build-system`
...
Installed 2 packages in 2ms
 + ccaas-data-tables==0.1.0 (from file:///.../shared/ccaas-data-tables)
 + click==8.5.0
$ ../../.venv/bin/my_script
entry point installed
```

The same fresh run with `[tool.uv]` instead of `[tools.uv]` installs and runs
the script without the warning. Running `uv sync` at the workspace root also
installs and runs the script without warning for either spelling.

Existing coverage in `crates/uv/tests/sync/sync.rs` does not exercise this
combination:

- `sync_scripts_workspace_member_not_packaged` selects a non-required member
  with `--all-packages`, expects the warning, and observes no member package
  installation.
- `sync_scripts_workspace_member_not_packaged_not_synced` syncs the root when
  the member is neither selected nor a root dependency and expects no warning.

Neither test selects from the child directory a member that is also required by
the workspace root, then verifies whether its entry point was installed.

## Fix

Fixed by making the entry-point warning use the same required-member package
decision as workspace requirement lowering. A workspace member required by
another member is installable without a build system unless it explicitly sets
`tool.uv.package = false`, so the warning now calls `is_package(false)` for
required members and retains the build-system requirement for other members.

The parent regression test
`sync_scripts_workspace_member_not_packaged_root_dependency` now expects the
required child to be installed without the contradictory warning and still
executes its generated entry point. The neighboring warning tests continue to
verify that genuinely virtual projects and workspace members warn, while an
unselected member does not. No distinct failure was found in another
integration-test module: `uv run` reuses the same sync implementation, and the
workspace lock and requirement-lowering coverage already records required
members as installable rather than virtual.

Focused validation succeeded with:

```console
cargo test --package uv --test sync sync_scripts_
cargo +stable fmt --all -- --check
cargo +stable clippy --package uv --test sync -- -D warnings
```

## Related

- astral-sh/uv#18388 (closed), "No warning printed about entrypoint installation
  being skipped for workspace members" — the historical counterpart for a
  workspace member whose scripts genuinely are skipped.
- astral-sh/uv#18389 (merged), "Warn when workspace member scripts are skipped
  due to missing build system" — added the workspace-member warning and the
  `sync_scripts_workspace_member_not_packaged` test, but does not cover the
  contradictory installed-entry-point case reproduced here.
- astral-sh/uv#14891 (closed), "Install entry points for workspace members" —
  establishes the related behavior that making a member a dependency of the
  workspace root causes that member to be installed.

Pull request: https://github.com/astral-sh/uv-dev/pull/2232
