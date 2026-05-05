# Plan: split `crates/uv`

## Rules

- `crates/uv` becomes a thin binary/dispatcher.
- `*-command` crates own CLI command implementation.
- Non-`*-command` crates own reusable utilities/domain logic.
- Use ~no re-exporting: import from the owning module directly.
- Do not preserve backwards compatibility for internal Rust APIs; update call sites directly.
- ESSENTIAL: do not change implementations/behavior during the split; move code and update paths only.
- Avoid a monolithic `uv-api` crate.

## Support crates

- `uv-cli-arguments`
  - Rename current `uv-cli`.
  - Clap args/enums/parsers/help metadata only.

- `uv-cli-types`
  - Small shared CLI control types.
  - Initially: `uv_cli_types::exit::ExitStatus`.

- `uv-cli-output`
  - Terminal-facing output only.
  - `Printer`, `OutputWriter`, progress reporters, diagnostics rendering, formatting helpers.

- `uv-child-process`
  - General child-process helpers.
  - `run_to_completion`.

- `uv-operations`
  - Shared high-level resolve/install operations.
  - Requirements reading, resolution, installation, changed-dist/changelog types, bytecode compilation, latest-version client, shared errors.
  - No Clap, no `ExitStatus`.
  - Initial mechanical move may keep resolve/install loggers here to avoid changing implementations; split terminal-only pieces to `uv-cli-output` after the operation/output seam is cleaner.

- `uv-project`
  - Project utilities only, not project commands.
  - Project errors, environment/interpreter discovery, script target handling, lock/sync shared helpers, install/lock targets.

## Command crates

- `uv-pip-command` — `uv pip ...`
- `uv-lock-command` — `uv lock`
- `uv-sync-command` — `uv sync`
- `uv-add-command` — `uv add`
- `uv-remove-command` — `uv remove`
- `uv-run-command` — `uv run`
- `uv-init-command` — `uv init`
- `uv-export-command` — `uv export`
- `uv-tree-command` — `uv tree`
- `uv-format-command` — `uv format`
- `uv-version-command` — `uv version`
- `uv-audit-command` — `uv audit`
- `uv-workspace-command` — `uv workspace ...`
- `uv-python-command` — `uv python ...`
- `uv-tool-command` — `uv tool ...` and `uvx`
- `uv-auth-command` — `uv auth ...`
- `uv-cache-command` — `uv cache ...`
- `uv-build-command` — `uv build` and build-backend entrypoints
- `uv-publish-command` — `uv publish`
- `uv-venv-command` — `uv venv`
- `uv-help-command` — `uv help`
- `uv-self-update-command` — `uv self update`, feature-gated
- `uv-pylock-command` — internal/pylock command implementation if kept separate

Existing domain crates remain as-is: `uv-python`, `uv-tool`, `uv-workspace`, `uv-cache`, `uv-auth`, `uv-publish`, `uv-audit`, etc.

## First safe commits

1. `uv-cli-output`
   - Move `Printer`, `OutputWriter`, formatting helpers, and progress reporters.

2. `uv-operations`
   - Move `pip::operations`, latest-version client, resolution marker/tag helpers, bytecode helper, and resolve/install loggers mechanically.

3. `uv-cli-types`
   - Move `ExitStatus`.

4. `uv-child-process`
   - Move `run_to_completion`.

5. `uv-project`
   - Move only shared script/project utility types first.

6. First small command extraction
   - Prefer `uv-workspace-command` as the first command crate.

## API style examples

Use owning modules directly:

```rust
uv_pip_command::install::install(...)
uv_lock_command::lock::lock(...)
uv_sync_command::sync::sync(...)
uv_add_command::add::add(...)
uv_project::script::ScriptTarget
uv_cli_types::exit::ExitStatus
uv_cli_output::printer::Printer
uv_cli_output::reporters::ResolverReporter
uv_operations::installation::install(...)
uv_child_process::run_to_completion(...)
```

Avoid umbrella imports/re-exports like:

```rust
uv_pip_command::install(...)
uv_project::*
uv_cli_output::*
```
