# BUG: `exclude-newer-package` is ignored with `index-exclude-newer`

Issue: astral-sh/uv#23103

## Summary

The reported failure is reproducible with the installed uv 0.13.6 on Ubuntu 24.04.5
LTS, using CPython 3.15.2 and the original `uv==0.13.7` requirement. The reporter
uses uv 0.13.7 on Fedora 44 with Python 3.15.2; that uv version and Fedora were
not directly tested.

The final PEP 723 example places `exclude-newer-package.uv = false` inside
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
but not cutoff scope or this diagnostic. Targeted searches under
`crates/uv/tests/` and `crates/uv-client/tests/it/` found no direct test for this
misplaced-key scenario.

No repository tests or builds were run. Runtime confirmation uses the installed
uv 0.13.6, not a checkout build or uv 0.13.7. All four command outcomes were
observed directly; the exact reported uv release and Fedora platform remain
untested.

## Supporting implementation evidence

- `docs/concepts/resolution.md` documents package exemptions under `[tool.uv]`
  and explicitly states that they take precedence over global and index values.
  `docs/concepts/indexes.md` repeats that precedence.
- `crates/uv-distribution-types/src/index.rs`, `IndexWire`, has an
  `exclude_newer` field but no `exclude_newer_package` field or
  `deny_unknown_fields`. This agrees with the observed silent acceptance of the
  misplaced key.
- `crates/uv-configuration/src/exclude_newer.rs`,
  `exclude_newer_package_for_index_with_source`, checks the package override
  before the index/global cutoff. The corrected-script result directly confirms
  this precedence for the reported command shape.
- `crates/uv-resolver/src/pubgrub/report.rs` consults the index cutoff for its
  cutoff hint when `fork_indexes` has an entry; otherwise it consults the
  package/global setting. `apply_dependency_source` in
  `crates/uv-resolver/src/resolver/mod.rs` populates those entries from package
  source/index constraints. This is consistent with the missing hint on the
  report's ordinary index-discovery path; no resolver fix was implemented or
  tested.

## Related

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
concerns settings that already produce a warning; this nested key produces none.

## Suggested maintainer response

The failure reproduces with the supplied layout on uv 0.13.6 and Python 3.15.2.
`exclude-newer-package` belongs under `[tool.uv]`, before `[[tool.uv.index]]`.
Moving that line makes `uv add 'uv==0.13.7' --script ...` succeed with the index
cutoff still enabled. Package exemptions do override index cutoffs when scoped
correctly. The unknown nested setting is silently ignored, and the failure
message omits the index cutoff while suggesting another PyPI index; those are
separate validation and diagnostic concerns for follow-up.
