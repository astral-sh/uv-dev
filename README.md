# `uv lock --upgrade-package` fails with "No solution found" on a lock the same command just wrote

Issue: astral-sh/uv#23142

Classification: bug

## Summary

On Linux x86_64 with CPython 3.12 and uv 0.12.15 (musl), the reporter runs package-specific upgrades for every non-Git package in a public paperless-ngx checkout. The first run succeeds, updating torch from 2.14.0 to 2.14.1. A subsequent `uv lock --python 3.12 --locked` accepts the resulting lock, but repeating the original upgrade command fails with “No solution found.”

The failing split is `python_full_version >= '3.15' and platform_python_implementation == 'PyPy' and sys_platform == 'darwin'`. The explanation says torch>=2.13.0 has no macOS-compatible wheels. The first run introduced this split among its resolution markers. Plain `uv lock` accepts the first-pass lock, and full `uv lock --upgrade` reportedly succeeds on both passes.

The closest discussions concern repeated-upgrade marker changes, discrepancies between discovered and pre-existing forks, and a separate marker-canonicalization problem. None establishes the same second-pass wheel-rejection failure or a confirmed fix.

## Report details

- Public repository: paperless-ngx/paperless-ngx, revision `5f17b1a310aa011f43b56474221d97f8d6bfe689`.
- Reported requirement: `torch>=2.13,<2.15`, sourced from a named `pytorch-cpu` index.
- The project lock contains multiple resolution forks and Git sources.
- The reproduction extracts unique package names whose lock sources do not contain a Git entry and passes one `--upgrade-package` argument for each name. The same argument list is reused on the second pass.
- Git sources are intentionally excluded so their pinned revisions remain unchanged. Full `--upgrade` is therefore a diagnostic comparison, not a verified replacement workflow.
- Expected behavior: the second package-specific upgrade succeeds, with no change or another valid lock.
- The reporter repeats upgrades because of the marker-ordering behavior discussed in astral-sh/uv#22040.

## Draft response

The second-pass resolution failure is distinct from the marker reordering tracked in astral-sh/uv#22040. Package-specific upgrades can reuse the lockfile's resolution forks, while --upgrade starts without them; this explains a difference between the code paths, but does not establish the cause of the failure.

Please retry the same two-pass reproduction with the latest uv release. If it still fails, attach the uv.lock produced by the successful first pass and verbose output from both passes. Keeping the Git revisions unchanged will help isolate the resolver behavior. We haven't confirmed a fix for this failure.

## Classification

The reported unchanged-input workflow succeeds once and then cannot resolve using the lock it just generated, while full upgrade succeeds repeatedly. That is a resolver correctness problem. Source confirms that package-specific upgrades can reuse stored forks while --upgrade discards them, but does not establish why this fork fails. The related issues have materially different outcomes or unconfirmed mechanisms, so duplicate classification is not supported. No recurrence of a previously fixed bug has been established.

Acceptance by `--locked` alone does not prove that a fresh resolution succeeds. The material correctness concern is the reported difference between repeated package-specific upgrades and repeated full upgrades on unchanged project inputs. Reproduction and confirmation of the responsible code remain downstream work.

The reporter's explanation that the second solve “loses the fallback” is a hypothesis. The wheel error and additional forks are reported observations; no specific fallback, index-selection defect, or marker-canonicalization defect has been confirmed.

## Related

- astral-sh/uv#22040 — Varying order of resolution-markers from the same `pyproject.toml` (issue, open). The reporter's linked issue includes the same two-pass --upgrade-package workflow on uv 0.12.15 with Git-pinned dependencies. Its runs succeed and reorder or simplify markers; it does not track the second-pass unsatisfiable resolution reported here.
- astral-sh/uv#6176 — Apply fork markers at fork time (issue, open). Tracks dependencies being solved in forks disjoint from their markers. Maintainers explicitly discuss instability between discovering fork points during resolution and starting with existing forks. This is relevant resolver context, but that mechanism has not been established for this wheel-rejection failure.
- astral-sh/uv#17747 — `uv lock` not working as expected in some cases when `upgrade_package` is set (issue, open). Another package-specific-upgrade report involving newly introduced forks and unstable lock output. Its --locked invocation rejects the new lock as out of date, whereas this report's --locked succeeds and the next upgrade fails to solve. The reporter confirmed a marker-canonicalization experiment helped that separate case.
- astral-sh/uv#17752 — Fix canonicalization of incompatible marker pairs compound disjunctions (pull request, open). The experimental fix linked from astral-sh/uv#17747 targets marker-tree canonicalization. A maintainer said it could not ship as-is; it remains unmerged. There is no evidence it fixes this issue's second-pass resolution failure.
- astral-sh/uv#21672 — Scope required-environment checks to the current fork (pull request, merged). Changed wheel-coverage checks to use the current resolver fork, allowing backtracking or an unsupported-environment error. Released in uv 0.12.14, before the reported 0.12.15. It is relevant historical context for fork-specific wheel rejection, not a confirmed cause or fix for this report.

## Supporting evidence

### Source and release timing

The uv 0.12.15 version of `crates/uv/src/commands/project/lock.rs` confirms:

- Lines 1227–1234 mark an existing lock unusable for full `--upgrade`.
- Lines 1348–1354 mark it preferable for package-specific upgrades after fork-coverage checks.
- Lines 999–1026 reuse fork markers from satisfying or preferable locks as initial universal resolver environments.

The checkout implements the same distinction in `crates/uv-lock-operations/src/validated_lock.rs` at lines 109 and 242, and `crates/uv-lock-operations/src/lock.rs` at lines 886–910. This supports investigating the reused-fork path; it does not identify the failing constraint.

The 0.12.x changelog places astral-sh/uv#21672 in uv 0.12.14, released September 15, 2026. The reported uv 0.12.15 was also released that day. Later changes include astral-sh/uv#21825 in 0.12.17 (Darwin baseline wheel boundaries) and astral-sh/uv#21835 plus astral-sh/uv#21836 in 0.12.18 (Python-fork wheel validation). Their descriptions do not confirm a fix for repeated package-specific upgrades, so a current-version retest is needed.

### Inspected alternatives

- astral-sh/uv#5180 describes a historical second-run collapse of forks and versions. Its maintainer explanation motivated storing and reusing fork markers. The referenced merged changes include astral-sh/uv#5480, astral-sh/uv#5481, and astral-sh/uv#5485. The new failure has not been shown to be a recurrence of that resolved instability.
- astral-sh/uv#20368 and merged astral-sh/uv#20369 concern memoization of impossible cross-variable platform markers and alternating dependencies under plain locking. That specific trigger is absent from the report.
- astral-sh/uv#9963 points to merged astral-sh/uv#10046 for PyTorch CPU local-version fallback. That fix considers a non-local release when it supports a missing platform and otherwise retains the local version. It does not establish the cause of this existing-lock failure.
- astral-sh/uv#21820 and merged astral-sh/uv#21835 concern a successful resolution selecting ONNX Runtime wheels unsuitable for an older Python fork, followed by installation failure. The new report fails during resolution on a subsequent upgrade.
- astral-sh/uv#23073 concerns lock validation accepting dependency-specifier violations after manual edits or merges. The reporter here says uv itself wrote the lock; no such edit is reported.
- astral-sh/uv#22805 and merged astral-sh/uv#22806 concern missing Android wheel-tag mappings. They do not explain a Darwin split.
- astral-sh/uv#4987 was resolved by supplying the private package index. astral-sh/uv#13373 was closed after correcting a workspace dependency source. Neither establishes the reported fork-dependent failure.

## Search coverage

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using literal errors and identifiers (--upgrade-package, No solution, macOS-compatible wheels, resolution-markers, pytorch-cpu, PyPy, paperless), repeated-run and own-output symptoms, and conceptual terms including idempotence, existing forks, preferences, unsatisfiable resolution, wheel coverage, and upgrades retaining Git pins. PR keyword searches returned no matches, so supplemented them with title filtering over 1,200 recent PRs across all states and direct inspection of referenced historical PRs. Read the strongest candidates and their comments and followed their linked fixes. Compared release notes and the 0.12.15 source with the checkout. Inspected but ruled out astral-sh/uv#5180 and its historical fork-persistence fixes as proof of the same regression; astral-sh/uv#20368 and astral-sh/uv#20369 concern impossible cross-variable markers; astral-sh/uv#9963 and astral-sh/uv#10046 concern CPU/local-version fallback; astral-sh/uv#21820 and astral-sh/uv#21835 concern installation after selecting wheels incompatible with an older Python fork. Also ruled out astral-sh/uv#23073, whose accepted invalid locks follow manual edits or merges, and astral-sh/uv#22805, whose regression is Android wheel-tag recognition. No existing discussion or fix was confirmed to cover this exact failure.

The report was decomposed before searching into: repeated-run failure; differences among package-specific upgrade, full upgrade, plain locking, and locked validation; fork and marker persistence; wheel availability in a non-current Python/implementation/platform split; and upgrading registry packages while retaining Git pins. Symptom searches were kept separate from possible-cause searches.

## Maintainer next steps

Request a retest of the unchanged two-pass reproduction on the current uv release. If it persists, obtain the first-pass lockfile and verbose logs for the successful and failing passes, then reduce the failing fork while retaining the original Git revisions. Compare the reused-fork solve with full upgrade before attributing the behavior to wheel coverage, marker canonicalization, or fallback selection.

## Verification and scope

This handoff is based on the report, live GitHub issue and pull request discussions, release notes, and source inspection. The project reproduction was not executed, and no fix was tested. No GitHub changes were made. Only this temporary README was authored; existing checkout changes were left untouched.
