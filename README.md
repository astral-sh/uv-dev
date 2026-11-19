# uv lock --check reports multiple matches when a dependency has no package entry

Issue: astral-sh/uv#23295

Classification: bug

## Summary

`uv lock --check` rejects a lockfile with a name-only dependency on `demo-util` and no `[[package]]` entry for that name, but says there are multiple matches rather than none. The reporter encountered this after a merge on uv 0.13.9. The same diagnostic was reproduced on the runner's installed uv 0.13.7 using a minimal schema-v1 lockfile, with no merge or network access required.

## Draft response

Thanks for the reproducer. We reproduced the diagnostic on Linux as well. `uv lock --check` should still reject this lockfile, but it should say `demo-util` has no corresponding package entry. A focused parser change and regression test could distinguish zero matches from genuine ambiguity. To repair the merged lockfile, start from an intact lockfile from the base branch and run `uv lock` with the merged project metadata.

## Classification

This is a confirmed diagnostic bug, not a request to accept or automatically repair invalid lockfiles. The reproduction establishes the incorrect diagnostic, not how the original merge lost the package entry. Prior triage did not find a duplicate or a fix for this distinction.

## Reproduction

**Outcome: reproducible.** Tested on Ubuntu 24.04 x86_64 with the installed uv 0.13.7 (`x86_64-unknown-linux-gnu`) and system CPython 3.12.3. The reporter used macOS 26.5.1 arm64, uv 0.13.9 (`6b2d39dcc`), and managed CPython 3.12.13; that exact environment was not retested.

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

**Existing test coverage (inspected, not run):** `lock_unsupported_version` in `crates/uv/tests/lock/lock.rs:31242` snapshots the same misleading inner cause for an absent `iniconfig` package, but uses `--frozen` with unsupported schema v2. `missing_dependency_source_version_ambiguous` in `crates/uv-lock/src/lock/mod.rs:11451` snapshots the multiple-match message for a name-only reference with two actual `a` entries in schema v1.

## Related

- astral-sh/uv#11701 (closed issue): Same missing-`source` message from the lockfile parser, but the attached lockfile has two `numpy` package entries and a name-only reference. A maintainer reproduced the parse failure with the existing lockfile, could not reproduce it from scratch, and closed the issue without a fix. This is true ambiguity, not a missing package entry.
- astral-sh/uv#18788 (open issue): The same check mode and a post-merge lockfile inconsistency, but the graph problem is the inverse: an unreferenced transitive package remains and the check succeeds. Here a reference remains without a package, and the check already fails.
- astral-sh/uv#23251 (merged pull request): Improves `--locked`/`--check` diagnostics when a parsed lockfile is stale and needs updating. It does not address the parse-time diagnostic reproduced here.

## Supporting evidence

In the checkout, `Lock::try_from` in `crates/uv-lock/src/lock/mod.rs:6888` builds a lookup containing only names with exactly one package entry. Names with multiple entries are removed; names with no entries never enter the lookup. `PackageIdForDependency::unwire` at `crates/uv-lock/src/lock/mod.rs:8041` emits `MissingDependencySource` on a lookup miss for a dependency without `source`, without distinguishing these cases. A dependency with an explicit source and version reaches the later check for dependency IDs without a locked package at `crates/uv-lock/src/lock/mod.rs:2791`, consistent with the control output.

## Search scope

Searched open and closed issues and open, closed, and merged pull requests with authenticated GitHub CLI searches. Literal searches used the parse error, `more than one matching package`, the missing `source` wording, `uv lock --check`, `demo-util`, and `0.13.9`. Conceptual searches covered missing and dangling package entries, ambiguous or unrecognized lockfile dependencies, and invalid lockfiles after merges. Fix-oriented checks covered parser and lock diagnostics, recent and merged pull requests, and candidate discussion and closure chains. No matching fix or duplicate surfaced.

The earlier report astral-sh/uv#11384 and its fix, astral-sh/uv#11400, concern optional versions for dynamic packages with conflicting extras, not zero package entries. The requests in astral-sh/uv#11185 and astral-sh/uv#16348 concern automatic merge resolution or explicit replacement of invalid lockfiles, and astral-sh/uv#5633 requests merge guidance; none tracks the incorrect diagnostic. The similar title of astral-sh/uv#19196 refers to merging configuration for dependency cooldowns, not to a missing lockfile graph node.
