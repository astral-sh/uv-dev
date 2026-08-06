# uv tool run ignoring installed version if newer available

Issue: astral-sh/uv#20981

Classification: bug

## Summary

The reporter installs `ty` with `uv tool install --exclude-newer="1 week" ty`, which selects
`ty==0.0.65`, then runs `uv tool run ty --version`. Instead of using that installed environment,
uv resolves and installs `ty==0.0.68` in an ephemeral environment. The report covers uv 0.12.1
and 0.12.2 on Ubuntu Server 24.04 with Python 3.14.6.

The reported behavior was reproduced with the installed uv 0.12.2 on Linux. In an isolated tool
directory and cache, the exact relative cutoff selected `ty==0.0.65`; the subsequent unversioned
tool run executed `ty==0.0.68`, while the persistent installation remained at 0.0.65. Two controls
confirmed that ordinary installed-tool reuse still works and that supplying the same cutoff to the
run reuses 0.0.65.

This conflicts with both the CLI documentation and the tool concepts documentation: an
unversioned, non-isolated `uv tool run` is supposed to use a compatible tool installed by
`uv tool install`. The repository's `tool_run_from_install` integration test also asserts this
precedence for an older installed version of Black.

No open issue or pull request was found that already tracks the `--exclude-newer`-specific failure.
The closest history is the original installed-tool reuse tracker astral-sh/uv#4742, its
implementation in astral-sh/uv#4750, and the later receipt-options gate added by
astral-sh/uv#10207.

## Reproduction

Outcome: reproducible.

Environment:

- uv 0.12.2 (`x86_64-unknown-linux-gnu`)
- Linux 6.17.0-1020-azure x86_64
- CPython 3.12.3 at `/usr/bin/python3`
- Reproduced on 2026-08-06 against the package index available to uv
- Fresh `UV_TOOL_DIR`, `UV_TOOL_BIN_DIR`, and `UV_CACHE_DIR` below `$RUNNER_TEMP`; managed Python
  downloads disabled with `UV_PYTHON_DOWNLOADS=never`

Minimal commands, with `$TEST_ROOT` denoting a fresh temporary directory:

```console
$ export UV_TOOL_DIR="$TEST_ROOT/tools"
$ export UV_TOOL_BIN_DIR="$TEST_ROOT/bin"
$ export UV_CACHE_DIR="$TEST_ROOT/cache"
$ export UV_PYTHON_DOWNLOADS=never
$ uv tool install --python /usr/bin/python3 --exclude-newer='1 week' ty
Resolved 1 package in 87ms
Installed 1 package in 2ms
 + ty==0.0.65
Installed 1 executable: ty
$ "$UV_TOOL_BIN_DIR/ty" --version
ty 0.0.65
$ uv tool run --python /usr/bin/python3 ty --version
Installed 1 package in 1ms
ty 0.0.68
$ uv tool list
ty v0.0.65
- ty
```

Thus, the unversioned, non-isolated run did not use the installed 0.0.65 environment and instead
created or used an ephemeral 0.0.68 environment. The installed tool itself remained unchanged.
The different Python patch version from the report does not prevent reproduction.

Two targeted controls isolate the relevant condition:

```console
$ uv tool run --python /usr/bin/python3 --exclude-newer='1 week' ty --version
ty 0.0.65
```

With the same cutoff option on the run, uv reused the installed version. In a second fresh tool
directory, `uv tool install --python /usr/bin/python3 'ty==0.0.65'` followed by the same unversioned
`uv tool run` also printed `ty 0.0.65`. This shows that installed-tool reuse works in the ordinary
case and fails for the reported install/run option mismatch.

Existing coverage does not cover this trigger. The integration test
`crates/uv/tests/tool/tool_run.rs::tool_run_from_install` installs `black==24.1.0` without
`--exclude-newer` and verifies that unversioned `uv tool run black --version` prints 24.1.0. It
also covers explicit-version, `--isolated`, and `--with` paths, but it does not install a tool with
resolver options that are absent from the subsequent run. The observed controls are consistent with
the reuse gate in `crates/uv/src/commands/tool/run.rs`, which compares current `ToolOptions` with the
saved receipt before checking installed-package compatibility; `exclude_newer` is part of
`ToolOptions` in `crates/uv-settings/src/settings.rs`.

## Draft response

Thanks for the clear reproduction. An unversioned `uv tool run ty` should reuse the installed
tool; only an explicit version request or `--isolated` should bypass it. The current implementation
compares the run's resolver/install options with the installed tool receipt, and `--exclude-newer`
is part of that receipt, so installing with it and then running without it skips the installed
environment and resolves an ephemeral one. That does not match the documented behavior, so we'll
treat this as a bug. Until it is addressed, invoking the installed `ty` executable directly will
use the version selected by `uv tool install`.

## Classification

This is a bug, not an enhancement or question. Installed-tool reuse is existing, intentional
behavior:

- astral-sh/uv#4742 requested that `uv tool run` use an already-installed compatible tool.
- astral-sh/uv#4750 implemented that behavior and tested compatible requests and the explicit
  `--isolated` opt-out.
- Current CLI and concepts documentation state that an installed version is used unless a version
  is requested or `--isolated` is passed.
- The current `tool_run_from_install` integration test installs a specific older version and
  verifies that an unversioned tool run uses it.

The reproduction controls align with the relevant implementation. Before reusing an installed
environment, `get_or_create_environment` requires the run's `ToolOptions` to equal the options saved
in the installed tool's receipt. `ToolOptions` includes `exclude_newer`. An installation made with
`--exclude-newer` and a subsequent run without that option therefore do not pass this gate, even
though the installed package satisfies the unconstrained request; the observed run used the newer
ephemeral version.

That equality gate was added by merged astral-sh/uv#10207 while implementing constraints and
overrides for `uvx`. It postdates the installed-tool reuse implementation. The report is therefore
an option-specific regression or uncovered edge case in established behavior. It should not be
centralized in closed astral-sh/uv#4742, and no open tracker for the same regression was found.

## Related

### astral-sh/uv#4742 — `uv tool run` should use already-installed tool (if possible)

State: closed.

This is the original tracker for the behavior at issue. It requested that `uv tool run` select an
already-installed compatible tool, and it was closed by astral-sh/uv#4750. It is historical context,
not an active duplicate of this option-specific regression.

### astral-sh/uv#4750 — Use already-installed tools in `uv tool run`

State: merged.

This pull request implemented installed-tool reuse. Its summary says the installed environment is
used when it satisfies the request, and review explicitly required coverage for compatible and
incompatible constraints, `--isolated`, and `--with`. This establishes the intended precedence that
astral-sh/uv#20981 violates.

### astral-sh/uv#10207 — Allow `--constraints` and `--overrides` in `uvx`

State: merged.

This pull request added the installed-tool receipt-options equality check to the reuse path. In the
current source, `exclude_newer` is one of the persisted `ToolOptions`, making this change directly
relevant to the reported trigger. The pull request did not itself track the present bug and should
not be treated as a duplicate.

## Search scope and ruled-out candidates

Authenticated GitHub searches covered open and closed issues and open, closed, and merged pull
requests. Literal searches included `uv tool run`, `uv tool install`, `installed version`, the exact
documentation wording, `exclude-newer`, `ty`, `ignores installed`, `latest version`, and combinations
of the install and run commands. Conceptual searches covered `uvx`, reuse or preference for an
already-installed tool, installed versus cached or ephemeral environments, isolation, receipt
options, configuration mismatches, index options, and resolution. Fix-oriented searches covered
the original installed-tool implementation, constraints and overrides, relative `exclude-newer`
timestamps in tool receipts, and prior cache-reuse fixes.

The following plausible candidates were inspected and ruled out as canonical trackers:

- astral-sh/uv#15824 concerned an explicit `@latest` request recreating a cached environment even
  when it resolved to the same version. It was fixed by astral-sh/uv#15827. That case concerns cache
  reuse under an explicit latest request, while this report concerns failure to prefer a persistent
  installed environment for an unversioned request.
- astral-sh/uv#17419 requests a new way to derive `exclude-newer` from a tool release date. It is an
  enhancement about reproducible selection, not installed-tool precedence.
- astral-sh/uv#19117 requests pylock input for tool runs. It concerns lockfile support rather than
  reuse of an existing installed environment.
- astral-sh/uv#18901 preserves relative timestamps in tool receipts for upgrades and outdated
  checks. It confirms that relative `exclude-newer` state is persisted, but it does not track the
  run-time reuse failure.

## Supporting repository evidence

- `docs/concepts/tools.md` says that once a tool is installed, `uvx` uses the installed version by
  default and documents explicit `@latest` and `--isolated` as ways to ignore it.
- `crates/uv-cli/src/lib.rs` gives the same installed-version rule in the generated CLI reference.
- `crates/uv/tests/tool/tool_run.rs` contains `tool_run_from_install`, which verifies reuse of an
  older installed version.
- `crates/uv/src/commands/tool/run.rs` checks equality between the current `ToolOptions` and the
  saved receipt before testing whether installed packages satisfy the request.
- `crates/uv-settings/src/settings.rs` includes `exclude_newer` and `exclude_newer_package` in
  persisted `ToolOptions`.
- astral-sh/uv#10207 introduced the receipt-options equality check on 2025-03-04; relative
  `exclude-newer` support and receipt persistence were subsequently expanded before the reported
  uv 0.12.1/0.12.2 reproductions.
