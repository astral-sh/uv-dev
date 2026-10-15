# Multiple test regressions in 0.13.1

Issue: astral-sh/uv#22584

Classification: bug

## Summary

The reporter encounters nine integration-test failures while testing uv 0.13.1 on Gentoo Linux amd64, and asks whether tests involving private repositories should run for downstream users.

The claims are:

- Private Git installation: `install_git_private_https_multiple_pat`, `install_git_private_https_pat`, `install_git_private_https_pat_at_ref`, and `install_git_private_https_pat_mixed_with_public`.
- Private Git dependency editing: `add_git_private_source`, `add_git_private_raw`, and the two `add_git_private_rate_limited_by_github_rest_api_403_response` / `429_response` tests.
- Frozen workspace synchronization: `sync_frozen_workspace_member_git_credentials`.

Only the multiple-PAT install failure has detailed output. Its success snapshot is replaced by an error: Git fetch exits 128, with a redirect warning, `could not read Username for 'https://github.com': terminal prompts disabled`, and `expected flush after ref listing`. uv exits 1 instead of installing the two expected fixture packages. The other tests are listed as failures without their individual logs.

No confirmed duplicate found. astral-sh/uv#19419 and its merged fix astral-sh/uv#19423 explain the frozen-sync test's origin; astral-sh/uv#8467 concerns the same install test but a different failure mode.

## Draft response

These tests intentionally access private Git fixtures and supply their own test credentials; they are enabled by the test-git feature. The Git fetch failure you showed is not an expected result. The frozen-sync test was added by astral-sh/uv#19423, but its failure does not yet establish that the earlier workspace-credential bug has returned. Please share your Git version, exact test command and enabled Cargo features, last passing uv version, and redacted output from one edit failure and the frozen-sync failure. That will help determine whether all three groups fail at the same stage.

## Classification

The report shows a success-expected integration test failing during Git fetch, alongside eight other reported failures. Source confirms that these tests intentionally access private fixtures, supply fixture credentials, and are gated by the default-enabled test-git feature. This is a test-suite correctness report, not merely a request for access or clarification. No existing item was established to track the same failure. Although a regression test for the previously fixed astral-sh/uv#19419 is affected, recurrence of that particular bug and causation by uv 0.13.1 remain unconfirmed.

Treat the listed failures as reported evidence and the detailed install failure as the demonstrated failure mode. There is no verified common root cause across the nine tests. In particular, the evidence does not establish expired/revoked fixture credentials, a Git-version change, a redirect/authentication change, or a regression in frozen workspace discovery. The issue had no comments or linked follow-up fix when inspected.

## Related

- astral-sh/uv#19419 (issue, closed) — Git credentials in workspace member `[tool.uv.sources]` not applied under `uv sync --frozen`. The reported sync_frozen_workspace_member_git_credentials test explicitly covers this historical bug. It produced the same Git username error when frozen sync omitted workspace-member credentials. It was fixed in May 2026; the new report also includes pip-install and edit failures, so recurrence of that specific mechanism is unconfirmed.
- astral-sh/uv#19423 (pull request, merged) — Apply workspace-member `[tool.uv.sources]` credentials under `uv sync --frozen`. Merged May 17, 2026, fixing astral-sh/uv#19419 and adding the exact frozen-sync test now reported failing. The maintainer's final implementation discovers existing workspace members; that behavior remains in the checkout. This is historical context, not a confirmed fix for the new failures.
- astral-sh/uv#8467 (issue, open) — Test `install_git_private_https_pat_at_ref` timed out. Names one of the same private-Git integration tests, but reports a 90-second timeout with no output in October 2024. The new example exits with a Git authentication error and a failed success snapshot. No evidence establishes a shared cause.

## Supporting evidence

- `crates/uv/Cargo.toml:155` enables `test-defaults`; the feature list beginning at line 169 includes `test-git`. Its definition at line 185 introduces the Git testing dependency. There is no separate private-Git feature in that list.
- `crates/uv/tests/pip_install/pip_install.rs:3891`, line 3916, line 3942, and line 3971 document the four reported install scenarios. Their attributes require `test-git`; three additionally exclude Windows. Their bodies call the fixture-token helper.
- `crates/uv/tests/project/edit.rs:295` documents private-source resolution with credentials omitted from the resulting project file. Line 385 documents the raw-source variant retaining credentials. Both are gated by `test-git` and supply fixture credentials.
- The rate-limit tests at `crates/uv/tests/project/edit.rs:475` and line 512 also require `test-git` and supply fixture credentials. They configure a local mock GitHub fast-path endpoint to return 403 or 429. Their names therefore do not establish that the reporter encountered actual GitHub API rate limiting.
- `crates/uv/tests/sync/sync.rs:18777` explicitly identifies the frozen-sync test as a regression test for astral-sh/uv#19419. It supplies fixture credentials, first locks the project, then forces an uncached reinstall under frozen sync. The report does not show which invocation fails.
- `crates/uv-project-commands/src/sync.rs:102` still selects `MemberDiscovery::Existing` for frozen sync. This matches the maintainer's final approach described in astral-sh/uv#19423; the PR's initial body describes an earlier implementation, so the maintainer comment and current source are the relevant evidence.
- astral-sh/uv#19423 merged on May 17, 2026, and its test-inventory comment names the exact frozen-sync test. `CHANGELOG.md:6` dates uv 0.13.1 to October 14, 2026. This establishes chronology, not the cause of the newly reported failures.
- Test structure, attributes, comments, and helper-call presence were inspected without displaying or decoding fixture credential values. No builds, test executions, or private-repository access checks were performed.

## Search scope and excluded candidates

Used authenticated gh to search open and closed issues and attempt open, closed, and merged PR searches. Searched the install, edit/rate-limit, and frozen-sync claims separately using exact test identifiers, uv-private-pypackage, 'could not read Username', 'expected flush after ref listing', and 0.13.1; broadened to private Git tests, test-git, internal:testing, credentials, Gentoo, and GitHub 403/rate limits. Kept possible token-expiry causes separate from symptom searches. PR search results were unreliable (empty results or issues despite PR filters), and some requests hit a rate limit; supplemented them by screening the 500 newest PR titles across all states and directly inspecting referenced fixes, comments, and issue timelines. Ruled out astral-sh/uv#15188 (snapshot filtering), astral-sh/uv#17509 (test environment/interpreter discovery), astral-sh/uv#1980 and astral-sh/uv#16895 (credential-helper side effects), and astral-sh/uv#10493 (publish prompting). Also inspected astral-sh/uv#22357, astral-sh/uv#22464, and astral-sh/uv#22519; none establishes the reported private-fixture failure. Historical sync discussions astral-sh/uv#7453 and astral-sh/uv#8529 do not establish a duplicate. No confirmed duplicate was found within this scope.

The most plausible exclusions were:

- astral-sh/uv#15188 includes the same Git username wording, but its confirmed problem was snapshot path filtering when system Python lived in `/usr/bin`; the new install example changes success into failure.
- astral-sh/uv#17509 concerns Gentoo test-environment scrubbing and Python discovery. Its comments confirm resolution after environment and HOME fixes; the new example reaches Git fetch.
- astral-sh/uv#1980 and its merged fix astral-sh/uv#16895 concern a different private-Git test writing credentials to a developer's global helper. They do not establish failure to authenticate the fixtures.
- astral-sh/uv#10493 is a publish unit test waiting for interactive input, whereas the new example exits with terminal prompts disabled.
- astral-sh/uv#22357 is an open URL-serialization refactor intended to preserve behavior. astral-sh/uv#22464 optimizes already-installed full-commit dependencies. astral-sh/uv#22519 isolates local repository fixtures. Their descriptions and changed-file lists do not establish a fix or shared mechanism for these failures.
- astral-sh/uv#7453 is an older Git-source credential-propagation bug reported fixed in 0.4.11. astral-sh/uv#8529 requests a better way to provide Git credentials. Neither tracks this downstream test-suite failure.

PR keyword-search inconsistencies limit confidence in a repository-wide negative result. The related list contains verified records, and the absence of a duplicate is qualified accordingly.

## Next maintainer action

Obtain the Git version, exact test invocation and feature selection, and last passing uv version. Request redacted logs for one edit test and the frozen-sync test to distinguish setup/lock failures from failures in the behavior each test targets. Compare the same environment across versions before attributing the failures to 0.13.1. Do not request credential values.

The requested handoff is recorded here. No GitHub changes were made, and no checkout files were modified.
