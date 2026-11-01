# uv audit does not respect UV_PYTHON and missing --python flag

Issue: astral-sh/uv#23034

Classification: bug

## Summary

The reporter uses uv 0.13.6 on Linux aarch64 with Python 3.13.18. In a CI matrix, `UV_PYTHON=3.13` overrides a project's `.python-version` pin of 3.11 for `uv lock`, but `uv audit` selects and downloads Python 3.11. Audit also rejects `--python 3.13`.

The report contains three distinct claims:

1. Interpreter selection ignores `UV_PYTHON`, and audit lacks the corresponding `--python` option.
2. Plain `--frozen` avoids interpreter discovery, but adding `--python-version` or `--python-platform` restores it. The target version does not override the interpreter selected from `.python-version`.
3. The advertised target filters do not narrow the audited dependency graph. A requirement on `requests==2.32.5; python_version < '3.12'` remains audited with `--python-version 3.13`, even after changing the version file to 3.13. The reporter compares this with correctly filtered `uv tree` output.

No exact duplicate found. astral-sh/uv#18506 and astral-sh/uv#18185 explain interpreter downloads and the frozen workaround; astral-sh/uv#18511 provides related subset-filtering history.

## Draft response

Your examples match the current implementation: audit has no --python/UV_PYTHON override, and the markers computed from --python-version and --python-platform are not used to select audited packages. The latter contradicts the documented behavior.

For now, your `uv audit --frozen --preview-features audit-command` workaround avoids interpreter discovery when neither target flag is supplied, but it does not provide a Python-specific audit. The normal locking/download behavior is explained in astral-sh/uv#18506.

The next step is to wire interpreter selection and target-marker filtering through audit, with regression coverage for both. Your reproductions already provide the necessary cases.

## Classification

Source confirms that audit supplies no explicit interpreter request, exposes no --python/UV_PYTHON binding, and computes but discards the markers from --python-version and --python-platform. The documented filtering behavior is therefore incorrect, independently of whether adding --python is viewed as an enhancement. Historical PRs concern universal defaults and group/extra filtering; no previously fixed target-filtering bug or exact duplicate was established.

An interpreter download during a normal audit is not by itself a bug: audit may update or validate the lockfile. Selecting the interpreter used for locking is also distinct from selecting the target environment whose dependencies should be audited. The established correctness problem is that explicit target flags promise filtering but do not apply it. The missing interpreter override is a separate omission.

There is no evidence that a previously released fix for either omission has regressed. The historical universal/frozen condition predates the reported version. The dependency-group fix discussed below addressed a different filter.

## Related

- astral-sh/uv#18506 — Roadmap: `uv audit` (issue, open). Maintainer comments explain that audit normally locks first and recommend --frozen to avoid interpreter downloads. This explains the workaround, but does not track ignored UV_PYTHON or ineffective target filters.
- astral-sh/uv#18185 — Make universal the default for `uv audit` (pull request, merged). Introduced the exact condition that skips interpreter discovery only for frozen audits without --python-version or --python-platform. It establishes the historical behavior, not a fix for interpreter overrides or target filtering.
- astral-sh/uv#18511 — Evaluate extras and groups when determining auditable packages (pull request, merged). Introduced lockfile traversal filtered by extras and dependency groups. Maintainer discussion supports auditing everything by default with explicit subset selection; the implementation does not supply Python/platform markers to that traversal.

## Supporting evidence

Evidence was inspected in checkout commit `564a2074b02eb9de5aaaa9491ed74357ade0332f`.

- `crates/uv-cli/src/lib.rs:5056`: `AuditArgs` exposes target version/platform options, but no interpreter request or `UV_PYTHON` binding. Its target-option documentation explicitly promises dependencies for the requested environment.
- `crates/uv-cli/src/settings.rs:3182`: `AuditSettings` carries `python_version` and `python_platform`, with no interpreter override.
- `crates/uv-project-commands/src/audit.rs:108`: universal mode means neither target option is present. Interpreter discovery is skipped only when both frozen and universal are true.
- `crates/uv-project-commands/src/audit.rs:133`: project discovery receives `None` as the explicit Python request. Script discovery likewise receives `None`.
- `crates/uv-python-discovery/src/project.rs:122`: without an explicit request, discovery consults `.python-version` before falling back to the project's Python requirement. This explains the supplied 3.11 versus 3.13 example.
- `crates/uv-project-commands/src/audit.rs:198`: `resolution_markers` computes `_markers`, which is not passed to `audit_lock`.
- `crates/uv-audit-operations/src/lib.rs:41`: `audit_lock` accepts no target markers and selects packages with `lock.auditable(extras, groups, |_| true)`.
- `crates/uv-lock/src/lock/mod.rs:3506`: audit traversal accepts extras, groups, and a package collection predicate, with no target marker environment. These findings confirm why target flags do not filter the package selection.
- In astral-sh/uv#18506, maintainer comments dated April 9, 2026 explain the locking step and recommend `--frozen` to avoid interpreter installation. That advice describes the universal path.
- The diff for astral-sh/uv#18185, merged February 24, 2026, introduces the same universal/frozen interpreter-discovery condition and retains the documented target flags.
- The diff for astral-sh/uv#18511, merged March 26, 2026, introduces `packages_for_audit` with extras/groups inputs and no target markers.
- `CHANGELOG.md:6` dates uv 0.13.6 to October 26, 2026. The report was opened November 1, 2026. The relevant historical changes therefore precede the reported release.

## Other candidates inspected

- astral-sh/uv#18968 initially resembles a container/platform-specific audit discrepancy, but its comments narrow the problem to `UV_CONFIG_FILE` suppressing project vulnerability-ignore configuration. It is not interpreter selection or marker filtering.
- astral-sh/uv#22089 reports a documented audit flag doing nothing, but specifically concerns `--no-default-groups`. Its merged fix, astral-sh/uv#22090 on September 30, 2026, changes group selection and adds a group-selection regression test. It does not implement Python/platform filtering.
- astral-sh/uv#18970, merged April 12, 2026, fixes script handling and collecting extras during audit traversal. Neither fix addresses this report.
- astral-sh/uv#18119 provides the original audit scaffolding and leads to the universal-default work and roadmap.
- astral-sh/uv#22252 and astral-sh/uv#22192, both merged October 7, 2026, move shared audit workflows into modules/crates. Their descriptions establish refactoring context, not a prior fix for target filtering.

## Search coverage and limits

Searched astral-sh/uv open and closed issues and all PR states using audit, UV_PYTHON, --python, python-version, python-platform, frozen, and unexpected argument; separately searched interpreter/download behavior and marker/filtering concepts without package or platform restrictions. Reviewed audit-area issues, comments, referenced PRs, historical diffs, and audit-titled entries from the latest 4,000 PRs. Ruled out astral-sh/uv#18968 (UV_CONFIG_FILE precedence), astral-sh/uv#22089 and its merged fix astral-sh/uv#22090 (dependency-group selection), and astral-sh/uv#18970 (script handling and extras traversal). Additional targeted searches hit GitHub's rate limit; no exact duplicate or previous fix for these gaps was found in the retrieved evidence.

The initial decomposition kept observable behavior separate from implementation hypotheses: interpreter precedence and rejected flags; frozen-mode downloads; and target-specific dependency selection. Literal searches included the option/environment names and the unexpected-argument fragment. Conceptual searches covered interpreter discovery/downloads, universal audits, markers, filtering, and subset selection. Closed issues, merged changes, roadmap comments, and referenced fixes were checked for regression history.

Some PR search queries returned no results despite known PRs being accessible directly. To supplement those searches, the latest 4,000 PRs across all states were listed and audit-related titles reviewed, spanning the original audit scaffolding through current changes. Later targeted searches returned rate-limit errors. The duplicate assessment is based on the retrieved evidence, not a claim of exhaustive search coverage.

## Maintainer next steps and validation

Wire the explicit interpreter request through audit's CLI, settings, and discovery paths, and pass the target marker environment into auditable dependency selection. Keep the universal default and the meaning of interpreter versus target options distinct.

Add focused regressions for the supplied interpreter-precedence case and target-specific dependency exclusion, including the frozen path. The existing module is `crates/uv/tests/build/audit.rs`; its parent module gates it on both `test-python` and `test-pypi`. Inspection found no explicit `UV_PYTHON`, `--python-version`, or `--python-platform` coverage in that module. Follow its explicit fixtures, mock audit service, and `uv_snapshot!` patterns, and check shared coverage before adding tests.

This handoff is based on source inspection and GitHub history. The Docker reproduction was not executed, and no build or tests were run. The reporter's exact vulnerability counts depend on the advisory service; the source-confirmed filtering defect does not depend on those counts. No checkout files or GitHub state were changed.
