# BUG: `exclude-newer-package` is ignored with `index-exclude-newer`

Issue: astral-sh/uv#23103

## Summary

The reported failure is reproducible with the installed uv 0.13.6 on Ubuntu 24.04.5
LTS, using CPython 3.15.2 and the original `uv==0.13.7` requirement. The reporter
uses uv 0.13.7 on Fedora 44 with Python 3.15.2; the released executable and Fedora
were not directly tested.

In the baseline, the final PEP 723 example places `exclude-newer-package.uv = false` inside
`[[tool.uv.index]]`. TOML therefore attaches the exemption to the index, where
that setting is unsupported and silently ignored. The index cutoff excludes the
requested release. Moving only the exemption into `[tool.uv]`, before the index
table, makes the same add command succeed. Supported package-over-index
precedence works in the runtime reproduction.

The original failure also reproduces the confusing diagnostic: it suggests
looking on a subsequent index, with PyPI shown as both indexes, and omits an
explanation of the active index cutoff. The configured PyPI entry does not
replace the implicit default index. The observed output does not establish a
dependency-confusion or index-strategy problem.

The recorded production fix in astral-sh/uv-dev#3092 rejects unknown index
fields, so this malformed script reports a configuration error before resolution.
It does not introduce index-scoped package exemptions or change supported
exemption precedence. That pull request remains open and unmerged; the validation
below establishes its tested behavior, not a released fix. A separate open
proposal, astral-sh/uv#23110, adds index-scoped exemptions and changes unpinned-index
cutoff hints. See the fix and upstream-proposal sections for their different scopes.

## Classification

Bug context: silent acceptance of an unsupported index-table setting and a
misleading/missing cutoff diagnostic. The resolution failure itself follows from
the misplaced exemption; the title should not be interpreted as evidence that
correctly scoped package exemptions are ignored.

No regression was established. The behavior already occurs in uv 0.13.6, and the
report does not identify a last known-good release. The 0.13.6 and 0.13.7 release
notes were inspected; they do not announce a change to this configuration's
precedence. No duplicate was identified in the related-issue investigation.

## Reproduction

**Outcome: reproducible**, with the configuration-scope qualification above.
The recorded outputs below describe the baseline behavior before the fix.

### Environment and isolation

- Executable: `/opt/hostedtoolcache/uv/0.13.6/x86_64/uv`, discovered on `PATH`;
  version output: `uv 0.13.6 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu 24.04.5 LTS, Linux 7.0.0-1017-azure, x86_64.
- Interpreter: CPython 3.15.2, downloaded using the installed uv into the temporary
  reproduction directory. Scripts retain `requires-python = ">=3.15"`.
- Registry: public PyPI. Its release metadata confirms uv 0.13.7 was uploaded
  starting at `2026-11-03T02:41:43.776511Z`, after the November 1 cutoff.
- Working directory and artifacts: `/tmp/uv-23103-_czmxkyh`. Interpreter storage,
  cache, configuration/data directories, subprocess home, and temporary files
  were isolated there. Commands used a minimal subprocess environment with
  keyring disabled and `TZ=UTC`. No script body was executed and no source
  distribution was built.

### Minimal failing fixture and command

Create `index_misplaced.py` with exactly this metadata:

```python
# /// script
# requires-python = ">=3.15"
# dependencies = []
#
# [tool.uv]
# preview-features = ["index-exclude-newer"]
#
# [[tool.uv.index]]
# name = "pypi"
# url = "https://pypi.org/simple"
# exclude-newer = "2026-11-01"
# exclude-newer-package.uv = false
# ///
```

The interpreter preparation command, with all storage redirected as described
above, was:

```console
uv python install 3.15.2 --no-config --no-bin
```

Run in the temporary fixture directory:

```console
uv add 'uv==0.13.7' --script index_misplaced.py --no-python-downloads
```

Observed exit status: **1**. Output:

```text
error: Failed to add dependencies
  cause: No solution found when resolving dependencies
  cause: Because there is no version of uv==0.13.7 and you require uv==0.13.7, we can conclude that your requirements are unsatisfiable.

hint: `uv` was found on https://pypi.org/simple, but not at the requested version (uv==0.13.7). A compatible version may be available on a subsequent index (e.g., https://pypi.org/simple). By default, uv will only consider versions that are published on the first index that contains a given package, to avoid dependency confusion attacks. If all indexes are equally trusted, use `--index-strategy unsafe-best-match` to consider all versions from all indexes, regardless of the order in which they were defined.

hint: If you want to add the package regardless of the failed resolution, provide the `--frozen` flag to skip locking and syncing
```

There was no warning about the unsupported nested key and no index-cutoff hint.
The script's dependency list remained empty after failure.

### Corrected fixture and controls

Create a separate `index_corrected.py` with the exemption under `[tool.uv]`:

```python
# /// script
# requires-python = ">=3.15"
# dependencies = []
#
# [tool.uv]
# preview-features = ["index-exclude-newer"]
# exclude-newer-package.uv = false
#
# [[tool.uv.index]]
# name = "pypi"
# url = "https://pypi.org/simple"
# exclude-newer = "2026-11-01"
# ///
```

```console
uv add 'uv==0.13.7' --script index_corrected.py --no-python-downloads
```

Observed exit status: **0**, with `Resolved 1 package in 2ms`. The script's
metadata gained `"uv==0.13.7"` in its dependency list. No index strategy or source
pin was needed.

Two separate control scripts used the same Python requirement and initially empty
dependency list. `global_cutoff.py` contained only
`[tool.uv] exclude-newer = "2026-11-01"`; `global_override.py` additionally set
`exclude-newer-package.uv = false` in that same table. Explicit invocations were:

```console
uv add 'uv==0.13.7' --script global_cutoff.py --no-python-downloads
uv add 'uv==0.13.7' --script global_override.py --no-python-downloads
```

The cutoff-only control exited **1** and explained that uv was filtered by
`exclude-newer`, including the requested release's upload time. The exempted
control exited **0**, resolved one package, and added the dependency. The report's
second and third examples have identical settings, so they represent one control.

With `TZ=UTC`, the global date-only cutoff was rendered as
`2026-11-02T00:00:00Z`; the report renders `2026-11-01T23:00:00Z`. Both timestamps
precede the release and this display difference does not affect the result.
Runtime results are saved as the four `*.result.json` files in the reproduction
directory.

### Existing test coverage and limits

Read `crates/uv/tests/lock/lock.rs`, test
`lock_exclude_newer_index_value` (gated by `test-universal`). It uses a local
PyPI proxy without upload timestamps, an explicit index, a source-pinned
`iniconfig>=2` dependency, and an index cutoff. Its initial snapshots assert
failure with an index-specific cutoff hint, both with and without the preview
flag. Its final fixture places `exclude-newer-package = { iniconfig = false }`
under `[tool.uv]` and asserts successful resolution. This covers supported
package-over-index precedence, but not the misplaced index-table key or the
ordinary, unpinned index-discovery diagnostic reproduced here.

Also read `crates/uv/tests/project/edit.rs`, test `add_script_settings`. Its
fixture sets inline `resolution = "lowest-direct"`, adds `anyio>=3`, and snapshots
the updated script and lockfile. It covers honoring inline script configuration,
but not cutoff scope or this diagnostic. The parent regression in
`crates/uv/tests/project/edit.rs` directly covers the misplaced-key scenario;
its updated expectation and validation are described below.

The original reproduction used the installed uv 0.13.6 and did not run
repository tests. All four command outcomes above were observed directly. The
fix investigation additionally builds and tests the checkout in the debug/test
profile; the exact released uv 0.13.7 executable and Fedora platform remain
untested.

## Fix

**Recorded implementation outcome: fixed in validation; not merged.**
The open proposal astral-sh/uv-dev#3092 adds Serde's `deny_unknown_fields` to
`IndexWire` in `crates/uv-distribution-types/src/index.rs`. The unsupported
`exclude-newer-package` field now produces a TOML parse error, consistent with
strict validation of the surrounding uv configuration, instead of being discarded
and letting dependency resolution report an unrelated-looking index failure.
The supported exemption remains under `[tool.uv]`.

The original regression from astral-sh/uv-dev#3091 was retained in
`crates/uv/tests/project/edit.rs`, renamed to
`edit::add_script_index_exclude_newer_package_invalid`, and updated to snapshot
exit status 2 and the unknown-field error. Its fixture still uses the reported
PEP 723 layout, preview feature, index cutoff, and misplaced exemption, with
`iniconfig==2.0.0` as a stable release after the cutoff. Its existing module-level
`test-python` and `test-pypi` feature gates remain applicable.

The original test passed while asserting silent acceptance and a misleading
resolution failure. Before changing production code, changing its expectation
to a configuration error made it fail with the original exit status 1 and PyPI
fallback hint. With unknown-field validation enabled, the updated regression
passes and identifies `exclude-newer-package` as an unknown index field. Serde
reports the enclosing `[tool.uv]` span for this flattened script configuration.

Scope inspection included the adjacent inline-script settings test, the existing
index-cutoff/override tests in `crates/uv/tests/lock/lock.rs`, the index serializer,
and the index-table producer in `crates/uv-project-edit/src/lib.rs`. The writer
emits supported index fields; internal `origin` metadata is skipped during
serialization. The existing lock fixtures correctly scope package exemptions and
do not need changes. No distinct additional failure from this cause was
demonstrated, so no additional regression scenarios were added.

Successful focused validation, all using unoptimized debug/test builds:

```console
cargo test --locked -p uv --test project edit::add_script_index_exclude_newer_package_invalid -- --exact
cargo test --locked -p uv --test project edit::add_script_settings -- --exact
cargo test --locked -p uv --test project edit::add_index -- --exact
cargo test --locked -p uv --test lock lock_exclude_newer_index -- --nocapture
cargo fmt --all
cargo fmt --all --check
git diff --check
```

The lock filter ran both `lock::lock_exclude_newer_index_disable` and
`lock::lock_exclude_newer_index_value`. They verify valid index overrides and
package-over-index precedence. `edit::add_index` exercises writing index
configuration and reading it on a subsequent add operation.

A separate corrected PEP 723 fixture was tested with `target/debug/uv` (version
0.13.7) and CPython 3.15.2, using isolated subprocess configuration and caches
under `/tmp/uv-23103-fix-coxrlqfn`. It places
`exclude-newer-package.iniconfig = false` under `[tool.uv]` while retaining the
index's `exclude-newer = "2022-01-01"`. These explicit producer/consumer
round-trip commands all exited 0:

```console
uv add 'iniconfig==2.0.0' --script script.py
uv lock --script script.py
uv lock --check --script script.py
```

The generated script retained the top-level exemption and acquired the pinned
dependency; the generated lockfile contains iniconfig 2.0.0. This confirms that
correctly scoped exemptions survive metadata writes and subsequent reads.

The pinned Rust toolchain lacked rustfmt, and its installation was read-only.
The matching official Rust 1.99.0 formatter component was downloaded and extracted
under `/tmp/uv-rustfmt-1pnbppyn`; its binaries were prepended to `PATH` for the
formatting commands. No repository toolchain configuration was changed.

This fix prevents the reported malformed configuration from reaching the
misleading resolver diagnostic. Cutoff-hint generation for otherwise valid,
unpinned index configurations has a separate cause and was not changed. No
repository manifest, lockfile, workflow, agent configuration, or GitHub state was
modified.

## Upstream proposal and remaining decision

The issue commenter identifies astral-sh/uv#23110 as a submitted fix. Its open,
unmerged status and proposed scope were independently checked against the pull
request body and diff. The diff adds an `exclude_newer_package` map to `Index` and
`IndexWire`, resolves an index's package override before its index-wide cutoff,
and uses that lookup in the resolver provider and latest-version client. It also
changes cutoff-hint generation to fall back to `available_indexes` when
`fork_indexes` has no package entry, directly targeting the diagnostic gap for
unpinned indexes.

This proposal would make the report's nested package-exemption syntax supported.
The recorded fix in astral-sh/uv-dev#3092 instead rejects that syntax as an unknown
index field. Maintainers therefore need to choose whether to extend index
configuration or retain the documented top-level exemption and reject misplaced
keys. The supplied discussion contains no maintainer decision between those
approaches. The new fix proposal does not change the bug classification or make
this issue a duplicate.

The inspected diff adds a unit test for parsing an index-scoped exemption and
falling back to the index cutoff for another package. It does not add an
end-to-end script-resolution or unpinned-index diagnostic regression. No tests or
builds of astral-sh/uv#23110 were run here; the successful integration tests and
runtime round trips recorded above apply to the rejection-based fix, not this
alternative. The documented top-level exemption remains the reproduced
workaround while both proposals are unmerged.

## Supporting implementation evidence

- `docs/concepts/resolution.md` documents package exemptions under `[tool.uv]`
  and explicitly states that they take precedence over global and index values.
  `docs/concepts/indexes.md` repeats that precedence.
- `crates/uv-distribution-types/src/index.rs`, `IndexWire`, supports
  `exclude_newer` but not `exclude_newer_package`. Its missing unknown-field
  validation caused the observed silent acceptance of the misplaced key. The
  fix adds `deny_unknown_fields` to this deserialization wire type.
- `crates/uv-configuration/src/exclude_newer.rs`,
  `exclude_newer_package_for_index_with_source`, checks the package override
  before the index/global cutoff. The corrected-script result directly confirms
  this precedence for the reported command shape.
- `crates/uv-resolver/src/pubgrub/report.rs` consults the index cutoff for its
  cutoff hint when `fork_indexes` has an entry; otherwise it consults the
  package/global setting. `apply_dependency_source` in
  `crates/uv-resolver/src/resolver/mod.rs` populates those entries from package
  source/index constraints. This is consistent with the missing hint on the
  report's ordinary index-discovery path. The recorded validation-only fix did
  not change or test resolver hints; astral-sh/uv#23110 separately proposes changing
  this path, as described above.

## Related

- astral-sh/uv#23110 (pull request, open and unmerged) — Proposed directly in
  response to this issue. Adds index-scoped package exemptions and an
  available-index fallback for cutoff diagnostics. This differs from rejecting
  unsupported index keys; its implementation has been inspected but not locally
  validated in this investigation.
- astral-sh/uv-dev#3092 (pull request, open and unmerged) — The recorded, tested
  fix rejects unknown index fields and retains top-level package exemptions. It
  leaves the separate unpinned-index cutoff diagnostic gap unchanged.
- astral-sh/uv-dev#3091 (parent regression pull request) — Added the script test
  demonstrating silent acceptance of the misplaced index exemption. The fix
  updates that same regression to require a configuration error.
- astral-sh/uv#18839 (pull request, merged) — Added `exclude-newer` to
  `[[tool.uv.index]]` on April 8, 2026, with package > index > global precedence.
  It does not add `exclude-newer-package` inside index tables. The existing
  precedence integration test above accompanies this supported configuration.
- astral-sh/uv#19749 (issue, closed) — Requested per-index cooldowns, including
  package exemptions taking precedence. A maintainer pointed to
  astral-sh/uv#18839 and the reporter confirmed it supplied the requested feature.
  This establishes intended precedence rather than tracking the current
  acceptance/diagnostic behavior.
- astral-sh/uv#5878 (issue, closed) — Historical confusing no-version errors for
  releases excluded by a cutoff. Comments identify astral-sh/uv#18217 and
  astral-sh/uv#18952 as adding cutoff hints. The current global-cutoff control
  displays those hints; the index-discovery diagnostic gap is distinct.
- astral-sh/uv#18952 (pull request, merged) — Improved cutoff hints on April 15,
  2026, including a matching release's publication date. Relevant diagnostic
  background, without establishing this index-discovery path as a regression.

Related searches also considered astral-sh/uv#19239 (transitive constraints),
astral-sh/uv#21211 and astral-sh/uv#21227 (uv check tool selection),
astral-sh/uv#19266 (pre-release hint), astral-sh/uv#12340 (version bounds), and
astral-sh/uv#22951 (promoting existing configuration warnings to errors). None
establishes a duplicate for the behavior reproduced here. The last discussion
concerns settings that already produce a warning. The baseline accepted this
nested key without warning; the proposed fix reports a configuration error.

## Suggested maintainer response

`exclude-newer-package` belongs under `[tool.uv]`, before `[[tool.uv.index]]`.
Moving the line makes the reported add operation succeed with the index cutoff
still enabled. Package exemptions correctly override index cutoffs when scoped
there. The proposed fix rejects the unsupported index-level field with a TOML
configuration error, avoiding silent acceptance and the misleading resolution
hint for this layout. Focused integration tests and a corrected-script
add/lock/check round trip pass.

Pull request: astral-sh/uv-dev#3092
