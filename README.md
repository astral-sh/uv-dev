# `uv tool upgrade --all` reports `Nothing to upgrade` when tool enumeration fails

Issue: astral-sh/uv#21058

Classification: bug

## Summary

The reported behavior is reproducible with uv 0.12.3. An invalid directory entry under
`UV_TOOL_DIR` makes `uv tool list` report a package-name validation error and exit with status 2,
but `uv tool upgrade --all` reports `Nothing to upgrade` and exits successfully.

The behavior does not depend on resolving a tool or starting Python: it occurs while uv enumerates
the tool directory. Source inspection after reproduction confirms that the all-tools upgrade branch
calls `InstalledTools::tools()` and uses `unwrap_or_default()`, so this enumeration error becomes an
empty collection. The subsequent empty-set branch emits the successful no-op message. In contrast,
`uv tool list` propagates the result of `InstalledTools::tools()`.

## Classification

This is a bug because failure to enumerate the installed tools is presented as a successful empty
tool set. The targeted reproduction confirms the user-visible discrepancy independently of source
inspection. No existing integration test covers an invalid top-level directory name in
`UV_TOOL_DIR` for `uv tool upgrade --all`.

## Reproduction

Outcome: reproducible.

Tested with the installed `uv` executable in an isolated runner temporary directory, with uv 0.12.3
(`x86_64-unknown-linux-gnu`), Linux 6.17.0-1020-azure x86_64, and Python 3.12.3. The report used the
same uv release on Darwin 25.5.0 arm64 with Python 3.14.0. The matching result on Linux and the fact
that the failure occurs before Python selection indicate that neither reported platform nor Python
version is required for this reproduction.

Minimal commands (with cache, tool bin directory, and Python installation directory also placed
under the same temporary root during the actual run):

```sh
repro_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/uv-21058.XXXXXX")"
mkdir -p "$repro_dir/tools/tool backup"
UV_NO_CONFIG=1 UV_TOOL_DIR="$repro_dir/tools" uv tool list
UV_NO_CONFIG=1 UV_TOOL_DIR="$repro_dir/tools" uv tool upgrade --all
```

Observed results:

```text
$ uv tool list
error: Not a valid package or extra name: "tool backup". Names must start and end with a letter or digit and may only contain -, _, ., and alphanumeric characters.
exit: 2

$ uv tool upgrade --all
Nothing to upgrade
exit: 0
```

Relevant existing coverage was inspected. `crates/uv/tests/tool/tool_upgrade.rs` contains
`tool_upgrade_empty`, which expects `Nothing to upgrade` for a genuinely empty directory, and
`tool_upgrade_not_stop_if_upgrade_fails`, which expects a nonzero result for a malformed receipt
after a valid tool name has been enumerated. `crates/uv/tests/tool/tool_list.rs` contains
`tool_list_missing_receipt` and the invalid-receipt portion of `tool_list_deprecated`; both concern
valid tool directory names with malformed or missing receipts. None exercises a directory name
that causes `InstalledTools::tools()` itself to return an error.

## Draft response

Thanks for the focused reproduction. This is reproducible with uv 0.12.3: an invalid directory name
under `UV_TOOL_DIR` makes `uv tool list` fail with status 2, while `uv tool upgrade --all` reports
`Nothing to upgrade` and exits 0. The all-tools upgrade path converts the enumeration error into an
empty collection. It should preserve the error instead, with an integration snapshot covering the
invalid directory entry.

## Related

- astral-sh/uv#18120 — A prior user-visible false `Nothing to upgrade` result caused by private
  index authentication failure during resolution, not tool-directory enumeration.
- astral-sh/uv#18246 — The merged fix for astral-sh/uv#18120; relevant precedent for surfacing an
  inability to determine upgrades, but it does not affect directory enumeration.
- astral-sh/uv#7294 — Discussion establishing that `uv tool upgrade --all` should report individual
  failures and return a failing status after enumerating tools.
- astral-sh/uv#7333 — Implementation for astral-sh/uv#7294 that accumulates per-tool upgrade errors;
  it does not cover failure of the initial enumeration.

## Search evidence

The relevant implementation and integration tests under `crates/uv/tests/tool/` were inspected.
Repository searches for the invalid-name message, invalid tool directories, `tool backup`, and the
all-tools no-op output found no test for this top-level enumeration failure. The related issues and
pull requests above remain relevant but do not duplicate the reported mechanism.
