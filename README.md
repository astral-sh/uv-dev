# `uv tool upgrade --all` reports `Nothing to upgrade` when tool enumeration fails

Issue: astral-sh/uv#21058

Classification: bug

## Summary

When `uv tool upgrade --all` encounters an invalid directory entry under `UV_TOOL_DIR`, it treats
the failed tool enumeration as an empty set, prints `Nothing to upgrade`, and exits successfully.
The same entry makes `uv tool list` report the invalid package-name error and exit nonzero.

The source confirms the discrepancy. The all-tools branch calls `InstalledTools::tools()` and uses
`unwrap_or_default()`, converting any top-level enumeration error into an empty vector. The next
branch interprets that empty result as a valid no-op. In contrast, `uv tool list` propagates the
result of `InstalledTools::tools()` with `?`.

No existing issue or pull request tracks this exact problem. The closest historical work concerns
other stages where `uv tool upgrade` must report an inability or failure rather than a successful
no-op: astral-sh/uv#18120 and astral-sh/uv#18246 address missing index authentication, while
astral-sh/uv#7294 and astral-sh/uv#7333 address failures after individual tools have been enumerated.

## Draft response

Thanks for the focused reproduction. The current all-tools path does convert an error from
`InstalledTools::tools()` into an empty tool set, then reports `Nothing to upgrade` and exits
successfully; `uv tool list` propagates the same enumeration error. That message and status are
incorrect. We should propagate the enumeration error in `uv tool upgrade --all` and add an
integration snapshot covering an invalid entry in `UV_TOOL_DIR`.

## Classification

This is a bug because existing behavior produces a misleading success message and zero exit status
after an operation required to determine the installed tools has failed. The mechanism is confirmed
by the checked-in source: `InstalledTools::tools()` returns a `Result`, but the all-tools upgrade path
uses `unwrap_or_default()` and therefore cannot distinguish an enumeration failure from no installed
tools. `uv tool list` already demonstrates the expected error-propagation behavior. No existing
tracker covers the same underlying problem, so this is not a duplicate.

## Related

- astral-sh/uv#18120 — Closed issue with the closest prior user-visible failure: `uv tool upgrade`
  incorrectly reported `Nothing to upgrade` when invalid private-index authentication prevented it
  from determining available upgrades. Its failure occurs during index resolution rather than
  installed-tool enumeration.
- astral-sh/uv#18246 — Merged fix for astral-sh/uv#18120. It preserves an authentication policy so
  inability to query the required index is surfaced as an error. This is relevant precedent for
  avoiding a false no-op, but it does not change tool-directory enumeration.
- astral-sh/uv#7294 — Closed discussion establishing that `uv tool upgrade --all` should report
  individual failures and return a failing status after attempting all tools. It assumes enumeration
  has already succeeded.
- astral-sh/uv#7333 — Merged implementation for astral-sh/uv#7294. It accumulates per-tool upgrade
  errors and returns failure, but the initial `InstalledTools::tools()` call remains outside that
  handling and currently converts its error into an empty set.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
used `Nothing to upgrade`, `Not a valid package or extra name`, `uv tool upgrade --all`,
`UV_TOOL_DIR`, `installed_tools`, and `unwrap_or_default`. Conceptual queries covered malformed,
corrupt, and stray tool-directory entries; invalid receipts; enumeration and read-directory
failures; silent or swallowed errors; and exit-status handling. Fix-oriented searches inspected
prior misleading no-op reports and their merged changes.

Several plausible results were ruled out. astral-sh/uv#8276 is a valid no-op caused by respecting an
installed version pin. astral-sh/uv#18522 and astral-sh/uv#18586 concern `uv tool list --outdated`
using settings inconsistent with upgrade. astral-sh/uv#19630 concerns recovery from a corrupt
receipt after a valid tool name has already been enumerated. None covers suppression of a top-level
enumeration error caused by an invalid directory name.
