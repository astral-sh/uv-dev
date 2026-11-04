# BUG: index-exclude-newer exclude-newer-package

Issue: astral-sh/uv#23103

Classification: bug

## Summary

The reporter uses uv 0.13.7 on Fedora 44 with Python 3.15.2 and runs
`uv add 'uv==0.13.7' --script <filename>` against PEP 723 scripts. A November 1,
2026 cutoff intentionally predates the requested release, whose reported upload time is
November 3. A global cutoff correctly rejects the version; a top-level package exemption
allows it. The second and third supplied examples have identical settings.

The failing example enables the index-exclude-newer preview feature, explicitly configures
PyPI, and moves both settings inside the index table. TOML therefore nests
`exclude-newer-package.uv = false` under the index, rather than under `tool.uv`.
The index schema has no such field and silently ignores it. Correctly scoped package
exemptions already override index cutoffs.

A separate diagnostic defect remains: this ordinary index-discovery path does not explain
the index cutoff. Instead, the output suggests considering a subsequent index, showing
PyPI as both the current and subsequent index. The script names PyPI without replacing the
implicit default index. No evidence establishes an actual dependency-confusion or
index-strategy problem.

astral-sh/uv#18839 and astral-sh/uv#19749 establish supported package-over-index precedence. astral-sh/uv#5878 and astral-sh/uv#18952 provide historical cutoff-diagnostic context. No duplicate found; the report exposes a silently ignored index-table key and a missing index-cutoff explanation.

## Draft response

Move `exclude-newer-package.uv = false` into `[tool.uv]`, before `[[tool.uv.index]]`, and leave `exclude-newer = "2026-11-01"` on the index. Package exemptions take precedence over index cutoffs, as implemented in astral-sh/uv#18839 and covered by an existing integration test.

Your example also exposes two problems: the unsupported index-level key is silently ignored, and the error omits the active index cutoff while suggesting an index-strategy change. Those validation and diagnostic issues need addressing. Please retry with the exemption under `[tool.uv]`; if resolution still fails, share the revised script and verbose output.

## Classification

The final script places exclude-newer-package inside an index table, while the supported setting belongs under tool.uv. Source and an existing integration test confirm that a correctly scoped package exemption overrides an index cutoff. However, IndexWire silently ignores the misplaced key, and the resolver's diagnostic path consults index cutoffs only for explicitly constrained package indexes, omitting the cutoff explanation during ordinary index discovery. These established validation and misleading-output defects warrant bug classification. No duplicate or regression of a previously working index-specific path was established.

The historical hint fixes predate the reported version, but the first example demonstrates
that the global-cutoff hint works. The available evidence establishes an uncovered
index-specific path, not a confirmed return of a previously fixed behavior. None of the
inspected open discussions tracks the same defect closely enough to centralize this report.

## Related

- astral-sh/uv#18839 (pull request, merged) — Add `exclude-newer` to `[[tool.uv.index]]`. Introduced index cutoffs on April 8, 2026, with explicit precedence: top-level exclude-newer-package > index exclude-newer > global exclude-newer. Its implementation and the existing lock_exclude_newer_index_value test establish the supported configuration; it does not add exclude-newer-package inside an index table.
- astral-sh/uv#19749 (issue, closed) — `exclude-newer-index`: per-index cooldown periods. The same reporter previously requested per-index cooldowns with package exemptions taking precedence. A maintainer pointed to astral-sh/uv#18839 and the reporter confirmed that it supplied the requested feature. This establishes intended precedence, rather than tracking the current silent acceptance or diagnostic defects.
- astral-sh/uv#5878 (issue, closed) — Make error caused by `--exclude-newer` clearer. Historical discussion of the same confusing 'no version' wording when a cutoff excludes a release. Later comments identify astral-sh/uv#18217 and astral-sh/uv#18952 as adding cutoff hints. The new report's global-cutoff example already shows those hints; its index-only diagnostic gap is distinct.
- astral-sh/uv#18952 (pull request, merged) — Improve `--exclude-newer` hints. Merged April 15, 2026, to explain when a requested version or range is excluded even though older versions remain available, including a matching release's publication date. Relevant to the missing cutoff explanation here, but it does not establish that ordinary index discovery ever received that explanation or that this is a regression.

## Supporting evidence

- `docs/concepts/resolution.md:849` places package exemptions under `[tool.uv]`;
  line 865 explicitly states that package-specific values take precedence over both global
  and index-specific values. `docs/concepts/indexes.md:324` repeats that precedence.
- `crates/uv-distribution-types/src/index.rs:737` defines `IndexWire` without an
  `exclude_newer_package` field or Serde's `deny_unknown_fields`. Its custom
  deserializer at line 762 uses that wire type. Unknown index keys are discarded.
- `crates/uv-configuration/src/exclude_newer.rs:459` checks the package override first;
  a disabled package cutoff returns no timestamp before consulting the index or global
  setting. `crates/uv-resolver/src/resolver/provider.rs:143` uses this effective cutoff.
- `crates/uv/tests/lock/lock.rs:42639` already contains
  `lock_exclude_newer_index_value`, gated by `test-universal`. Its final scenario
  places the exemption under `[tool.uv]`, configures an index cutoff, pins the package
  to that index through sources, and snapshots successful resolution. This covers
  precedence with correctly scoped configuration.
- `crates/uv-resolver/src/pubgrub/report.rs:881` retrieves the index cutoff only when
  the package has a `ForkIndexes` entry; otherwise it checks only package/global
  configuration. `crates/uv-resolver/src/resolver/mod.rs:3476` populates those entries
  from explicit package source/index constraints. The reporter configures an index
  without such a package constraint, exposing the missing index-only cutoff hint.
- `crates/uv-resolver/src/pubgrub/report.rs:1310` independently emits the unchecked-index
  hint when later indexes exist. This explains the displayed hint but does not establish
  that changing index strategy is the appropriate remedy.
- The maintainer response in astral-sh/uv#19749 identifies astral-sh/uv#18839 as the
  implemented per-index feature, and the reporter agrees. The PR description explicitly
  defines package > index > global precedence.
- Comments on astral-sh/uv#5878 identify astral-sh/uv#18217 and astral-sh/uv#18952 as
  the historical cutoff-hint improvements. Both merged before this November report.

## Corrected configuration and next steps

Keep the package exemption outside the index table:

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

Retry the original add command with this layout. If it still fails, obtain the revised
script and verbose output before attributing any failure to package/index precedence.

For follow-up implementation, investigate validation of unsupported index keys and cutoff
hint generation for a package discovered through an ordinary configured index. The existing
precedence test uses a source-pinned index and does not cover that diagnostic trigger.
Any added integration coverage should use focused, named scenarios in the relevant
existing module and the appropriate test feature gate.

## Search scope and excluded candidates

Used authenticated gh to search astral-sh/uv open and closed issues and open, closed, and merged PRs. Literal searches covered index-exclude-newer, exclude-newer-package, exclude-newer, 'Because there is no version', 'subsequent index', and 'index-specific'; conceptual searches covered per-index cooldowns, package exemptions and precedence, ignored configuration, unknown index fields, cutoff diagnostics, and script resolution. Searches removed package/version/platform details and separately checked 0.13.7 and historical fixes. PR searches returned no matches; relevant PRs were found and inspected through issue/comment reference chains, including astral-sh/uv#19749 → astral-sh/uv#18839 and astral-sh/uv#18949 → astral-sh/uv#5878 → astral-sh/uv#18217 / astral-sh/uv#18952. Ruled out astral-sh/uv#19239 (transitive dependency constraints, acknowledged by its reporter), astral-sh/uv#21211 and astral-sh/uv#21227 (standalone ty selection by uv check), astral-sh/uv#19266 (pre-release hint), astral-sh/uv#12340 (available-version bounds), and astral-sh/uv#22951 (promoting existing parse warnings to errors). No existing item was found tracking this index-table validation and ordinary-index cutoff diagnostic combination.

astral-sh/uv#22951 concerns configuration that already emits a parse warning; a maintainer
describes compatibility reasons for warnings and supports considering a strict mode.
Here the unknown nested index key produces no warning, so that discussion does not
establish that silent acceptance is correct or track the same validation failure.

## Verification limits

Parsed all four report metadata blocks with Python's `tomllib`, without executing the
scripts. This confirmed that only the fourth nests the package exemption inside an index.
Reviewed repository documentation, resolver/deserializer source, existing integration
snapshots, and the linked GitHub discussions. No uv binary was available, so the corrected
script was not executed and no build or test suite was run. Runtime confirmation of that
script remains a follow-up step.
