# uv lock --check reports multiple matches when a dependency has no package entry

Issue: astral-sh/uv#23295

Classification: bug

## Summary

`uv lock --check` correctly rejects a lockfile with a dependency on `demo-util` and no `[[package]]` entry for that name, but the affected parser says there are multiple matches rather than none. The reporter encountered this after a merge on uv 0.13.9. The diagnostic was reproduced on the runner's installed uv 0.13.7 and the unpatched uv 0.13.9 checkout using a minimal schema-v1 lockfile, with no merge or network access required. The upstream fix, astral-sh/uv#23304, distinguishes missing package entries from genuine ambiguity and closed the issue on November 19, 2026.

## Draft response

Thanks for the reproducer. We reproduced the diagnostic on Linux as well. The fix still rejects this lockfile but reports that `demo-util` has no matching package entry. To repair the merged lockfile, start from an intact lockfile from the base branch and run `uv lock` with the merged project metadata.

## Classification

This is a confirmed diagnostic bug, not a request to accept or automatically repair invalid lockfiles. The reproduction establishes the incorrect diagnostic, not how the original merge lost the package entry. Prior triage did not find a duplicate or an earlier fix for this distinction.

## Reproduction

**Outcome: reproducible.** Tested on Ubuntu 24.04 x86_64 with the installed uv 0.13.7 (`x86_64-unknown-linux-gnu`) and system CPython 3.12.3. The reporter used macOS 26.5.1 arm64, uv 0.13.9 (`6b2d39dcc`), and managed CPython 3.12.13; that exact environment was not retested.

A separate commenter reported the same diagnostic and exit code 2 on uv 0.13.8 on Linux using the reporter's exact files. That run was not independently verified.

Use a clean temporary project and run with Python 3.12 or newer on `PATH`:

```sh
uv_bin=$(command -v uv)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/uv-23295.XXXXXXXX")
mkdir -p "$scratch/home" "$scratch/project" "$scratch/tmp"
cd "$scratch/project"
cat > pyproject.toml <<'EOF'
[project]
name = "demo"
version = "1.0"
requires-python = ">=3.12"
dependencies = ["demo-util"]
EOF
cat > uv.lock <<'EOF'
version = 1
requires-python = ">=3.12"

[[package]]
name = "demo"
version = "1.0"
source = { virtual = "." }
dependencies = [{ name = "demo-util" }]
EOF
env -i PATH="$PATH" HOME="$scratch/home" TMPDIR="$scratch/tmp" \
    XDG_CACHE_HOME="$scratch/cache" XDG_CONFIG_HOME="$scratch/config" \
    XDG_DATA_HOME="$scratch/data" XDG_STATE_HOME="$scratch/state" \
    UV_CACHE_DIR="$scratch/uv-cache" "$uv_bin" lock --check --offline --verbose
```

The command exits 2. After the interpreter-selection logs, it prints:

```text
error: Failed to parse `uv.lock`
  cause: Dependency `demo-util` has missing `source` field but has more than one matching package
```

As a control, adding `version = "1.0"` and `source = { registry = "https://pypi.org/simple" }` to the dependency reference while still omitting its package entry also exits 2. It gives the accurate cause: ``For package `demo==1.0 @ virtual+.`, found dependency `demo-util==1.0` with no locked package``. Both lockfiles should be rejected; the name-only case should identify the missing package just as the explicit-reference case does.

The unpatched debug build also reproduced the multiple-match error when the reference included a registry source but no version: it reported a missing `version` field with multiple matches despite the same absent package entry. The nearby `lock_unsupported_version` integration test covered the original missing-source symptom for an absent `iniconfig` package under `--frozen` with unsupported schema v2.

## Fix

**Outcome: fixed upstream.** The merged lockfile reader distinguishes names with zero, one, or multiple package entries. When source or required version inference fails because the name is absent, it reports ``Dependency `demo-util` has no matching package entry in the lockfile`` for the reported fixture. Genuine ambiguity retains the multiple-match diagnostic; the invalid lockfile is still rejected.

The internal fix in astral-sh/uv-dev#3143 also distinguishes these cases without changing the serializer or the handling of dynamic source-tree versions and fully specified dependency IDs. The validation below applies to that patch; the upstream pull request updates its own `lock_unsupported_version` snapshot for the missing-package diagnostic.

Both parent regressions in `crates/uv/tests/lock/lock.rs` passed before the fix while reporting false ambiguity. After their snapshots were updated, both failed on the misleading message. The related `lock_unsupported_version` snapshot failed for the same reason; its unsupported-schema error remains unchanged. The sync and export integration fixtures were checked for the same assumption; their parse-error tests exercise other validation and share this parser.

**Focused validation:**

- Five debug-profile integration tests passed: `lock_check_missing_package_without_source`, `lock_check_missing_package_without_version`, `lock_unsupported_version`, `lock_preserves_noncanonical_lock`, and `lock_regenerates_marker_specific_local_extra`.
- Seven focused `uv-lock` dependency lookup tests passed, covering unique names, genuine ambiguity, and dynamic versions.
- A fresh `uv lock --offline` followed by `uv lock --check --offline` passed with two platform-specific local versions of the same dependency. Removing the source and version from one dependency edge while retaining both package entries still produced the multiple-match error and exit 2.
- Rust formatting and `git diff --check` passed.

## Related

- astral-sh/uv#23304 (merged pull request): Fixes the reported parse-time diagnostic by tracking ambiguous names separately from absent names. GitHub records this pull request as closing astral-sh/uv#23295.
- astral-sh/uv#11701 (closed issue): Same missing-`source` message from the lockfile parser, but the attached lockfile has two `numpy` package entries and a name-only reference. A maintainer reproduced the parse failure with the existing lockfile, could not reproduce it from scratch, and closed the issue without a fix. This is true ambiguity, not a missing package entry.
- astral-sh/uv#18788 (open issue): The same check mode and a post-merge lockfile inconsistency, but the graph problem is the inverse: an unreferenced transitive package remains and the check succeeds. Here a reference remains without a package, and the check already fails.
- astral-sh/uv#23251 (merged pull request): Improves `--locked`/`--check` diagnostics when a parsed lockfile is stale and needs updating. It does not address the parse-time diagnostic reproduced here.
- astral-sh/uv-dev#3142 (parent regression pull request): Covers the name-only reference and the reference with a registry source but no version. Both tests now assert the missing-package diagnostic.

## Supporting evidence

Before the fix, `Lock::try_from` in `crates/uv-lock/src/lock/mod.rs` removed ambiguous names from the package lookup, so an absent name produced the same lookup miss. `PackageIdForDependency::unwire` treated either miss as ambiguity when it could not infer the source or required version. Both the canonical and general TOML readers use this conversion, and regular dependencies, optional dependencies, and dependency groups share the same edge reader. A fully specified reference can still reach the later check for dependency IDs without a locked package, as in the control run. The writer already elides the source and version only when the package name is unique.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with authenticated GitHub CLI searches. Literal searches used the parse error, `more than one matching package`, the missing `source` wording, `uv lock --check`, `demo-util`, and `0.13.9`. Conceptual searches covered missing and dangling package entries, ambiguous or unrecognized lockfile dependencies, and invalid lockfiles after merges. Fix-oriented checks covered parser and lock diagnostics, recent and merged pull requests, and candidate discussion and closure chains. The initial triage found no pre-existing fix or duplicate.

The earlier report astral-sh/uv#11384 and its fix, astral-sh/uv#11400, concern optional versions for dynamic packages with conflicting extras, not zero package entries. The requests in astral-sh/uv#11185 and astral-sh/uv#16348 concern automatic merge resolution or explicit replacement of invalid lockfiles, and astral-sh/uv#5633 requests merge guidance; none tracks the incorrect diagnostic. The similar title of astral-sh/uv#19196 refers to merging configuration for dependency cooldowns, not to a missing lockfile graph node.
