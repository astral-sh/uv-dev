# Crate layout

`uv` owns binary initialization, argument dispatch, logging, and the registry that collects error
hints. Command implementations live in their owning `*-command` crate. Shared logic lives in support
crates; callers import directly from the owning module without compatibility re-exports.

```text
uv (initialization and dispatch)
|
+-- uv-*-command (command implementations)
|   |
|   +-- uv-project (shared project/environment workflows)
|   |   `-- uv-operations (resolution and installation)
|   `-- uv-operations
|
+-- uv-cli-settings ----> uv-cli-arguments
`-- uv-cli-arguments      (Clap arguments and parsers)

Shared command support:
  uv-cli-types      exit status and command-selection types
  uv-cli-output     terminal output and progress reporters
  uv-cli-report     installation reports
  uv-cli-error      command error classification
  uv-child-process  process execution and environment files
```

The diagram shows selected dependencies. Command crates depend on the support and existing domain
crates they use directly.

## Support crates

- `uv-cli-arguments`: the former `uv-cli` crate, containing Clap arguments, enums, parsers, and help
  metadata.
- `uv-cli-settings`: CLI, environment, and filesystem configuration reconciliation, resolved
  settings, and HTTP client configuration.
- `uv-cli-types`: `ExitStatus`, script paths, and initialization, Python upgrade, and tool
  invocation control types.
- `uv-cli-output`: `Printer`, `OutputWriter`, progress reporters, formatting, and shell
  configuration output.
- `uv-cli-report`: installation report rendering that consumes operation results.
- `uv-cli-error`: user/unexpected error classification and operation error conversion.
- `uv-child-process`: child-process execution and environment-file loading.
- `uv-operations`: requirements reading, resolution, installation, changed-distribution/changelog
  types, bytecode compilation, latest-version lookup, and shared errors. This crate has no Clap
  arguments or `ExitStatus`. Resolve/install loggers remain alongside the operations they report.
- `uv-project`: project errors, environment/interpreter discovery, script targets, lock validation,
  installation targets, synchronization helpers, malware checks, and shared audit rendering.

The dispatcher supplies diagnostic callbacks to build and tool-upgrade handlers so their error
reports can include hints from the complete command error registry without a dependency cycle.

## Command crates

- `uv-pip-command`: `uv pip ...`.
- `uv-lock-command`: `uv lock`.
- `uv-sync-command`: `uv sync`.
- `uv-add-command`: `uv add`.
- `uv-remove-command`: `uv remove`.
- `uv-run-command`: `uv run`.
- `uv-init-command`: `uv init`.
- `uv-export-command`: `uv export`.
- `uv-tree-command`: `uv tree`.
- `uv-format-command`: `uv format`.
- `uv-check-command`: `uv check`.
- `uv-upgrade-command`: `uv upgrade`.
- `uv-version-command`: `uv version`.
- `uv-audit-command`: `uv audit`.
- `uv-workspace-command`: `uv workspace ...`.
- `uv-python-command`: `uv python ...`.
- `uv-tool-command`: `uv tool ...` and `uvx`.
- `uv-auth-command`: `uv auth ...`.
- `uv-cache-command`: `uv cache ...`.
- `uv-build-command`: `uv build` and build-backend entrypoints.
- `uv-publish-command`: `uv publish`.
- `uv-venv-command`: `uv venv`.
- `uv-help-command`: `uv help`.
- `uv-self-update-command`: `uv self update`, enabled through `uv`'s `self-update` feature.
- `uv-pylock-command`: shared pylock command support.

Existing domain crates, including `uv-python`, `uv-tool`, `uv-workspace`, `uv-cache`, `uv-auth`,
`uv-publish`, and `uv-audit`, continue to own their domain implementations.

## Imports

Use owning modules directly:

```rust
uv_pip_command::install::pip_install(...)
uv_lock_command::lock::lock(...)
uv_sync_command::sync::sync(...)
uv_add_command::add::add(...)
uv_project::environment::ScriptEnvironment
uv_cli_types::exit::ExitStatus
uv_cli_output::printer::Printer
uv_cli_output::reporters::ResolverReporter
uv_operations::installation::install(...)
uv_child_process::run_to_completion(...)
```
