# `uv check --script` ignores `exclude-newer-package` exemptions when selecting `ty`

Issue: astral-sh/uv#21211

Classification: bug

## Summary

The report shows `uv check --script` selecting `ty` 0.0.70 under a global seven-day
`exclude-newer` cutoff even though the script's inline `[tool.uv]` configuration explicitly sets
`exclude-newer-package = { ty = false }`. A cutoff-free `uvx ty@latest` selects 0.0.73. The expected
semantics are established by astral-sh/uv#16854: a package value of `false` disables the global
cutoff for that package.

The current source confirms the mismatch. `ResolverSettings` retains the full `ExcludeNewer`
value, including package overrides, but `crates/uv/src/commands/project/check.rs` extracts only
`settings.resolver.exclude_newer.global` and passes its timestamp to `ty::run`. The standalone
binary resolver in `crates/uv/src/commands/project/check/ty.rs` therefore has no way to observe the
`ty = false` override. This is not specific to the reported package versions or macOS platform.

No existing issue or pull request tracks this exact defect. The closest history establishes the
configuration contract, the origin of the affected command path, and a separate locked-tool
selection path for scripts.

## Draft response

Thanks for the clear reproduction. This is a bug. astral-sh/uv#16854 added `false` as a
package-specific opt-out from the global `exclude-newer` cutoff, but the standalone `ty` path in
`uv check` currently reduces the resolved settings to the global timestamp before selecting a
version. That means the `ty = false` entry cannot affect this selection.

This is separate from the locked direct-dependency selection added in astral-sh/uv#19989, since
this script does not declare `ty`. The next step is to pass `ty`'s effective package cutoff—including
a disabled cutoff—to the standalone resolver and add integration coverage for `uv check --script`.

## Classification

`bug` is the appropriate classification because uv applies the global setting while discarding a
more-specific override whose documented and implemented purpose is to take precedence. The source
confirms the setting is parsed and retained before being reduced to the global timestamp at the
`uv check`/`ty` boundary.

This is not a duplicate: searches found no open or closed issue and no open, closed, or merged pull
request tracking the same `uv check` omission. It is also not a regression of the general exemption
feature. astral-sh/uv#16854 predates `uv check`, and astral-sh/uv#19605 introduced the command with
the global-only extraction already present.

## Related

- astral-sh/uv#16854 — **Allow disabling `exclude-newer` per package** (merged pull request). It
  implemented the exact `PACKAGE=false` opt-out and confirms that a package-specific disabled value
  overrides the global cutoff. It concerns the general resolver feature, not a prior fix for
  standalone tool selection in `uv check`.
- astral-sh/uv#19605 — **Add `uv check` to run `ty` from uv** (merged pull request). Its diff
  introduced standalone `ty` selection by extracting only `settings.resolver.exclude_newer.global`.
  The same global-only boundary remains in current source, so this is the origin of the affected
  path rather than a separate tracker for the bug.
- astral-sh/uv#19989 — **Use locked dependency selection for `uv check --script`** (merged pull
  request). This is the closest prior work on choosing `ty` for a PEP 723 script, but it handles a
  directly declared and locked `ty` dependency. The new reproduction declares no dependencies and
  reaches the standalone resolver instead.

## Search evidence

Authenticated searches covered open and closed issues and open, closed, and merged pull requests.
Literal queries included `exclude-newer-package`, `exclude-newer`, `uv check --script`, `ty`,
`Using ty`, and `preview-features=check-command`. Conceptual queries covered ignored or silently
ignored overrides, package-specific and per-package cutoffs, configuration propagation, standalone
tool selection, and selecting the latest checker. Fix-oriented searches used `exclude_newer`, the
`uv check` label and command history, and removed the incidental versions and platform from the
query.

The strongest apparent match, astral-sh/uv#19239, was ruled out. It reported an older package after
using an exemption with `uv pip compile`, but the reporter later confirmed that non-exempt transitive
dependencies constrained the selected version; the override itself was not ignored. Open reports
about persisting exemptions, configuration merging, structured bypasses, glob support, and stale
entries concern different resolver policies or lockfile behavior. No later fix for the `uv check`
global-only handoff was found.
