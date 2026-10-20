# Unhelpful warning when using `--no-project` with `UV_FROZEN` set

Issue: astral-sh/uv#22755

Classification: bug (diagnostic usability; warning policy requires a maintainer decision)

## Summary

The reported warning is reproducible. With `UV_FROZEN=1` inherited from the environment, `uv run --no-project test.py` prints:

```text
warning: `--frozen` has no effect when used alongside `--no-project`
```

The script still runs successfully. The warning occurs both in an otherwise empty directory and beside a minimal `pyproject.toml`. It names a command-line flag even though frozen mode was enabled only through the environment. Removing `UV_FROZEN` or passing `--no-frozen` eliminates the warning.

The reporter used uv 0.13.3 on Linux x86_64. Initial runtime reproduction used the installed uv 0.13.1 on Linux x86_64. The three parent regression tests subsequently passed against a debug build of the uv 0.13.3 checkout, directly confirming all three warnings there as well. The report does not claim an upgrade regression or an execution failure.

## Classification

Retain the diagnostic-usability bug classification. The observed behavior establishes that an inherited setting produces the reported warning, not that ignoring the setting or issuing a warning is itself incorrect. The reporter asks whether environment-derived settings should be exempt from these no-effect warnings. Suppression versus source-aware wording remains a maintainer decision.

`UV_NO_SYNC=1` also produces flag-only wording. `UV_LOCKED=1` already identifies its environment source, but still warns. These extensions should be considered separately rather than assuming identical attribution behavior for all three settings.

## Reproduction

Outcome: **reproducible**.

### Environment and scope

- Reported: uv 0.13.3 (`x86_64-unknown-linux-gnu`), Linux `6.8.0-142-generic` x86_64 GNU/Linux; Python version listed as N/A. The report does not specify the value of `UV_FROZEN`; the reproduction uses the supported enabled value `1`.
- Executed: uv 0.13.1 (`x86_64-unknown-linux-gnu`) from `/opt/hostedtoolcache/uv/0.13.1/x86_64/uv`, resolved from `PATH`.
- Host: Linux `6.17.0-1022-azure` x86_64, glibc 2.39; CPython 3.12.3 at `/usr/bin/python3`.
- Initial source review: commit `502f93c`, the uv 0.13.3 version bump. Focused baseline validation also built and tested the parent regression checkout at `72f83210ebe1d4d165c73b2f152429379aa0ab4b` in the debug test profile; only the three parent regression tests ran.
- All reproduction files and caches are under `/tmp/uv-22755-qpf_x17h`. Each invocation used a fresh, explicitly constructed environment with no inherited virtual environment or other uv settings. `UV_NO_CONFIG=1`, `UV_OFFLINE=1`, `UV_PYTHON_DOWNLOADS=never`, and `UV_PYTHON=/usr/bin/python3` disabled configuration discovery and downloads and selected the existing interpreter. `TMPDIR`, `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, and the XDG cache/config/data directories all pointed inside the temporary directory.

### Minimal input and commands

`test.py` is an ordinary script without PEP 723 metadata:

```python
print("script ran")
```

With the isolation settings above, run in the temporary working directory:

```sh
UV_FROZEN=1 uv run --no-project test.py
```

Observed exit status: `0`. Standard output:

```text
script ran
```

Standard error:

```text
warning: `--frozen` has no effect when used alongside `--no-project`
```

This first reproduced without any project file. The same command produced identical output after adding only:

```toml
[project]
name = "warning-repro"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []
```

There were no workspace members, dependency groups, sources, lockfile, virtual environment, or script dependencies. The runs created neither `uv.lock` nor `.venv`.

### Controls and related settings

Each command below ran separately beside that project file, with only the indicated setting enabled. All exited `0` and printed `script ran`.

| Command | Standard error |
| --- | --- |
| `uv run --no-project test.py` with `UV_FROZEN` unset | Empty |
| `uv run --no-project --frozen test.py` with `UV_FROZEN` unset | `warning: ` followed by the same `--frozen` warning reproduced above |
| `UV_FROZEN=1 uv run --no-project --no-frozen test.py` | Empty |
| `UV_LOCKED=1 uv run --no-project test.py` | `warning: ` followed by `` `UV_LOCKED=1` has no effect when used alongside `--no-project` `` |
| `UV_NO_SYNC=1 uv run --no-project test.py` | `warning: ` followed by `` `--no-sync` has no effect when used alongside `--no-project` `` |

The `--no-frozen` override is therefore an observed per-invocation workaround. Raw command arguments, environment overrides, exit codes, stdout, stderr, and version metadata are retained in `/tmp/uv-22755-qpf_x17h/results.json`.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for `UV_FROZEN`, `UV_LOCKED`, `UV_NO_SYNC`, and the no-project warning. Read the relevant project-run fixtures and assertions:

- `crates/uv/tests/project/run.rs`, `run_no_project` (line 3512): creates a Python 3.12 project depending on `anyio`, runs it to create a virtual environment, then verifies ordinary and isolated `--no-project` execution. Its final snapshot explicitly passes `--locked`, expects exit status 0, and checks the `--locked`/`--no-project` warning. It does not exercise environment-derived frozen mode.
- `crates/uv/tests/project/run.rs`, `run_no_sync_env_var` (line 2838): verifies that `UV_NO_SYNC=1` allows project execution without a lockfile, skips installing requirements after locking, and allows importing installed dependencies after syncing. It does not use `--no-project` or assert this warning.

The parent regression pull request astral-sh/uv-dev#2893 adds `run_no_project_frozen_env_var`, `run_no_project_locked_env_var`, and `run_no_project_no_sync_env_var` in `crates/uv/tests/project/run.rs`. Each uses an ordinary script with no project or dependencies, asserts successful execution, and snapshots the corresponding unwanted warning. All three passed unchanged in focused baseline validation. The project-run module is gated by both `test-python` and `test-pypi` in `crates/uv/tests/project/main.rs`. No test files were modified during the fix investigation.

## Implementation evidence

The inspected uv 0.13.3 source is consistent with the runtime observation on uv 0.13.1:

- `crates/uv-cli/src/settings.rs:653`: `resolve_frozen` distinguishes `FrozenSource::Cli` from `FrozenSource::Env`; `--no-frozen` prevents the environment fallback.
- `crates/uv-cli/src/settings.rs:783`: `RunSettings::resolve` resolves locked, frozen, and no-sync settings from CLI arguments and the environment. The no-sync value is reduced to a boolean at line 863.
- `crates/uv-project-commands/src/run.rs:601`: in the ordinary-script/non-PEP-723 path, the no-project warning block checks `frozen.is_some()` and emits hard-coded `--frozen` wording at line 612. It does not branch on the setting's source. No-sync similarly uses hard-coded `--no-sync` wording.
- `crates/uv-project-commands/src/run.rs:609` and `crates/uv-settings/src/resolved.rs:50`: the locked warning formats `LockedSource`; the environment variant renders as `UV_LOCKED=1`.
- `crates/uv-cli/src/lib.rs:3407`: the CLI documents `--no-frozen` as overriding `UV_FROZEN`.

These source observations explain the diagnostic path in the checkout. The published uv 0.13.3 executable was not tested, but the parent regression tests now confirm the same behavior in a debug build of its source plus the regression tests.

## Related

The persisted issue context identifies these relationships; no additional GitHub search was performed during reproduction or fix investigation:

- astral-sh/uv-dev#2893 (parent regression pull request, draft) — Adds three passing integration tests that assert the reported warnings for inherited frozen, locked, and no-sync settings.

- astral-sh/uv#5977 (pull request, merged) — Introduced warnings for project-specific settings passed to non-project `uv run` commands, including the exact frozen/no-project warning, in August 2024. It did not address suppressing warnings for inherited environment settings.
- astral-sh/uv#5856 (issue, closed) — Original request behind astral-sh/uv#5977, asking for warnings when project options are ignored outside a project. This explains the warning's purpose but does not settle the policy for environment defaults.
- astral-sh/uv#17259 (issue, closed) — Related confusing diagnostics when `UV_FROZEN=1` is inherited by `uv lock` or `uv sync`; concerned missing-lockfile errors and was addressed by astral-sh/uv#17318.
- astral-sh/uv#17318 (pull request, merged) — January 2026 change making missing-lockfile errors identify `UV_FROZEN` or `UV_LOCKED` as their source. It left the no-project warning unchanged; the available evidence does not establish a regression of that fix.

## Prior search context

The existing handoff records searches of open and closed issues and pull requests in astral-sh/uv, including exact warning text, combinations of `--no-project` and the three environment settings, ignored options, and environment-aware diagnostics. No duplicate was found in that search.

Excluded candidates were astral-sh/uv#10743, whose similar warning concerns dependency-group discovery and a missing `[project]` table; astral-sh/uv#9772, which requests warnings about resolution options ignored by frozen mode; and astral-sh/uv#5855, which concerns ignored `[tool.uv]` settings in inline script metadata. They do not describe this environment-derived no-project warning.

## Suggested next step

Decide whether inherited project-only settings should produce no-effect warnings under `--no-project`, or whether the warnings should identify the environment variable. Review frozen, locked, and no-sync separately because their source tracking and wording differ. Once the desired policy is decided, update the three parent regression snapshots in the project-run integration module and retain meaningful explicit-flag coverage.

## Fix

Outcome: **not_fixed**. The checkout's source and regression tests remain unchanged because the requested replacement behavior requires a diagnostic-policy decision.

The root cause is confirmed: the ordinary-script no-project branch warns whenever frozen, locked, or no-sync mode is enabled, without excluding environment-derived settings. Frozen retains its source but the warning hard-codes `--frozen`; locked formats its source and already correctly names `UV_LOCKED=1`; no-sync is reduced to a boolean in CLI settings before the command consumes it. Thus correcting attribution alone would not address the locked case, while suppressing all three warnings would change the intentional policy of reporting ignored settings. The report asks whether environment defaults should be exempt, and neither the persisted issue information nor the parent regression pull request establishes that policy. No speculative desired-behavior snapshots or production changes were made.

The neighboring paths and assertions were inspected:

- `run_no_project` explicitly retains a warning for `--locked`. The parent tests already represent the three demonstrated environment-derived cases, so additional ordinary-script invocations would duplicate their coverage.
- `run_no_sync_env_var` verifies project environment reuse and skipped synchronization. Its assertions concern effective no-sync behavior rather than no-effect diagnostics and are not contradicted by the confirmed cause.
- `run_pep723_script` intentionally warns about ignoring `UV_LOCKED` when the script has no lockfile. Inline scripts use a separate branch, and their lock/sync settings can affect script environments. Clearing inherited settings globally during argument resolution would risk changing those semantics; this is not a demonstrated additional failure of the ordinary-script no-project warning.
- `crates/uv/tests/sync/sync.rs`, `sync_lock_flags_override_environment`, checks CLI/environment precedence using a stale lockfile, source-aware warnings, and unchanged lockfile contents. Those producer/consumer assertions are consistent with the source distinction and do not establish a no-project suppression policy. No additional round-trip tests were warranted without a production change.

Successful focused baseline validation:

```sh
cargo test --locked -p uv --test project run::run_no_project_ -- --nocapture
```

The debug test profile completed successfully: **3 passed, 0 failed, 666 filtered out**. Each parent regression printed `script ran`, exited 0, and matched its existing warning snapshot. This validates the reported behavior, not a fix. No entire-suite run, release build, source edit, test edit, or GitHub change was made. Formatting was unnecessary because no Rust files changed.

Only this issue-context README was updated during the fix investigation; Cargo also generated ordinary build and test artifacts. The source checkout remains unchanged.
