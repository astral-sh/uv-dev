# Multiple test regressions in 0.13.1

Issue: astral-sh/uv#22584

Classification: bug

## Summary

The reporter lists nine private-Git integration-test failures on Gentoo Linux amd64 with uv 0.13.1 and asks whether these tests should run for downstream users. Only `pip_install::install_git_private_https_multiple_pat` has detailed output: a success snapshot becomes uv exit 1 after Git fetch exits 128, with a redirect warning, `could not read Username for 'https://github.com': terminal prompts disabled`, and `expected flush after ref listing`.

**Downstream result:** mgorny reports that downgrading to `curl < 8.23` fixes all the reported test failures and plans to pass the issue to Gentoo's Git maintainer (astral-sh/uv#22584, comment ID 6199894472, October 15, 2026 at 18:55:44 UTC). This validates the downgrade as a workaround in the reporter's environment and strongly supports the identified Git/libcurl compatibility bug. No uv 0.13.1 regression has been demonstrated.

**Local reproduction outcome: not_reproducible.** All nine reported test scenarios passed against the published uv 0.13.1 binary on Ubuntu x86_64 with Git 2.55.0 and Python 3.12.3. The detailed multiple-PAT scenario also passed against the installed uv 0.13.0 binary. These earlier local results remain valid and are separate from the reporter's downstream downgrade verification.

Maintainer charliermarsh confirmed that fixture credentials are intentionally shipped in the repository and that the private repositories test behavior rather than protect confidential contents (astral-sh/uv#22584, comment ID 6190830431, October 15, 2026 at 11:19:58 UTC). This confirms the intent behind the inspected tests supplying their own fixture credentials: downstream users are not expected to obtain separate private-repository access. In the inspected source, the tests are gated by the default-enabled `test-git` feature. The two rate-limit tests simulate GitHub API responses locally and do not demonstrate actual GitHub rate limiting. The reporter has now verified the curl downgrade workaround; exact Git and curl package revisions and Git patch status were not supplied.

**Maintainer direction:** charliermarsh plans to gate these private-Git tests behind an additional feature beyond `test-git` (astral-sh/uv#22584, comment ID 6190687672, October 15, 2026 at 11:11:51 UTC). This is a stated implementation plan. The supplied discussion does not specify a feature name, default-enablement policy, implementation PR, or release target. The additional gate would give separate control over these tests; it does not establish why the authenticated fetches failed downstream.

**Upstream mechanism:** curl 8.23.0 removes credentials from the reported effective URL, which Git without the cited fix can mistake for a redirect, discarding authentication and prompting again. This source-backed mechanism matches the original redirect warning and username failure, and the reporter's successful curl downgrade corroborates its relevance. The Git fix or a downstream backport has not yet been reported tested in that environment.

## Classification

Retain the bug classification, with the evidence now pointing to an external Git/libcurl compatibility bug affecting the integration tests rather than a demonstrated uv 0.13.1 regression. The reporter confirms that a curl downgrade resolves all reported failures. The expected behavior remains successful installation, dependency editing, and frozen synchronization; private fixtures alone do not explain a failure.

No confirmed duplicate within uv was found. The upstream Git/libcurl mechanism is source-backed and now supported by the reporter's downgrade result; the precise downstream Git build and patch status remain unverified. There is no evidence that astral-sh/uv#19419 has regressed or that fixture credentials expired. The separate feature-gating plan does not change the classification or the recorded local reproduction outcome.

## Reproduction

### Environment and method

- Ubuntu 24.04.5 LTS, Linux x86_64; Git 2.55.0; system CPython 3.12.3.
- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`.
- Comparison executable: `/tmp/uv-22584-j5bpqa/tools/bin/uv`, installed from the published `uv==0.13.1` wheel using the installed uv. Both executables reported their expected versions and `x86_64-unknown-linux-gnu`.
- Temporary reproduction root: `/tmp/uv-22584-j5bpqa`. Environments, package caches, Cargo cache and build output, Python cache, configuration, and test directories were confined there.
- Copied the nine existing test bodies into a temporary Rust harness with their original setup, command arguments, feature gates, and snapshots. Each function body was compared with the `0.13.1` release tag and was identical. The harness uses the checkout's `uv-test` helpers and the installed uv binaries; it does not build or run a replacement uv binary.
- Built the harness with Rust 1.99.0 in the development test profile. Git system/global configuration and credential helpers were disabled for isolation. The original helpers supplied fixture authentication internally. The runner retained only test status and fixed error indicators, not credential-bearing command output.
- Tests were invoked explicitly, one named scenario per invocation, with fresh `TestContext` directories and package caches. The multiple-PAT case was run on both uv versions; the other eight cases were run on 0.13.1.

The reporter did not provide a last known-good uv version. Version 0.13.0 is a comparison version, not a confirmed downstream baseline. This harness does not recreate Gentoo's package recipe, downstream patches, full Cargo test invocation, or inherited Git configuration. The later reporter-verified `curl < 8.23` workaround was not executed in this local harness.

### Commands and artifacts

The setup used the installed uv to create temporary environments and install the comparison binary:

```sh
uv venv --python /usr/bin/python3 /tmp/uv-22584-j5bpqa/tools
uv pip install --python /tmp/uv-22584-j5bpqa/tools/bin/python --only-binary :all: --no-deps 'uv==0.13.1'
```

The isolated environment used by these commands is defined in `/tmp/uv-22584-j5bpqa/control.py`. The harness is at `/tmp/uv-22584-j5bpqa/workspace/crates/probe`; its build command was:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/tmp/uv-22584-j5bpqa/cargo-home CARGO_TARGET_DIR=/tmp/uv-22584-j5bpqa/cargo-target cargo test --no-run --manifest-path /tmp/uv-22584-j5bpqa/workspace/crates/probe/Cargo.toml
```

Representative executed commands, covering the detailed failure and both other test groups:

```sh
python3 /tmp/uv-22584-j5bpqa/run_existing_test.py authenticated-multiple-0131 0.13.1 pip_install::install_git_private_https_multiple_pat
python3 /tmp/uv-22584-j5bpqa/run_existing_test.py authenticated-multiple-0130 0.13.0 pip_install::install_git_private_https_multiple_pat
python3 /tmp/uv-22584-j5bpqa/run_existing_test.py authenticated-add-source-0131 0.13.1 edit::add_git_private_source
python3 /tmp/uv-22584-j5bpqa/run_existing_test.py authenticated-sync-0131 0.13.1 sync::sync_frozen_workspace_member_git_credentials
```

The runner supplies `CARGO_BIN_EXE_uv` and a temporary `CARGO_MANIFEST_DIR`, then executes the compiled harness with `--exact <test-name> --nocapture`, capturing output internally. Sanitized results are stored as `authenticated-*.json`; release-source comparisons are in `release-test-comparison.json`.

### Observed results and existing coverage

Every listed scenario passed its original snapshots on uv 0.13.1. None produced the reported authentication failure, redirect warning, or flush error. Test setup and assertions were read before making the following coverage claims.

| Existing integration-test file | Test | Behavior verified |
| --- | --- | --- |
| `crates/uv/tests/pip_install/pip_install.rs` | `install_git_private_https_multiple_pat` | Installs both private fixture packages with separate PATs; expected two-package success snapshot and installed-package check pass. Also passes on uv 0.13.0. |
| `crates/uv/tests/pip_install/pip_install.rs` | `install_git_private_https_pat` | Installs the private fixture using its PAT and checks installation. |
| `crates/uv/tests/pip_install/pip_install.rs` | `install_git_private_https_pat_at_ref` | Installs the private fixture pinned to the test's specific commit. |
| `crates/uv/tests/pip_install/pip_install.rs` | `install_git_private_https_pat_mixed_with_public` | Installs private and public fixtures together without incorrectly sharing authentication. |
| `crates/uv/tests/project/edit.rs` | `add_git_private_source` | Adds the private Git dependency, checks credential-free project and lockfile snapshots, and successfully runs subsequent frozen sync. |
| `crates/uv/tests/project/edit.rs` | `add_git_private_raw` | Adds with `--raw-sources`, checks the expected project and lockfile snapshots, and successfully runs subsequent frozen sync. |
| `crates/uv/tests/project/edit.rs` | `add_git_private_rate_limited_by_github_rest_api_403_response` | A local mock returns 403 for the GitHub fast path; adding the private Git dependency still succeeds. |
| `crates/uv/tests/project/edit.rs` | `add_git_private_rate_limited_by_github_rest_api_429_response` | A local mock returns 429, including the expected retries; adding the private Git dependency still succeeds. |
| `crates/uv/tests/sync/sync.rs` | `sync_frozen_workspace_member_git_credentials` | Root and nested member both depend on the private fixture, with its authenticated source declared only in the member. `uv lock` succeeds, then root `uv sync --frozen --reinstall --no-cache` performs a fresh fetch and installs successfully. |

The workspace test has no dependency groups. Its root/member source placement and forced uncached frozen execution were retained exactly; a simplified public-only workspace was not used as evidence for its success.

Credential-free controls used `uv pip install --python <temporary-venv>/bin/python 'uv-private-pypackage @ git+https://github.com/astral-test/uv-private-pypackage'` with separate empty caches on 0.13.0 and 0.13.1. Both failed as expected with uv exit 1, Git exit 128, and terminal prompts disabled. A 0.13.1 `.git`-suffix variant failed the same way. Neither produced the redirect warning or flush error. The public fixture installed successfully on 0.13.1. These controls establish baseline behavior only: intentionally omitting authentication does not reproduce a failure of tests that supply authentication.

### Downstream verification and remaining details

The reporter states that downgrading to `curl < 8.23` resolves all the test issues, covering the reported install, edit, and frozen-sync failures. This is a reporter-verified workaround, not an independently repeated local experiment. No exact downgraded curl package revision, Git build, test invocation, or post-downgrade logs were supplied. The reporter plans to involve Gentoo's Git maintainer.

For validating a lasting downstream fix, identify the Git package and the libcurl used by its HTTP transport, check whether the cited Git fix or an equivalent backport is present, and rerun the affected tests with curl 8.23.0 and patched Git. Broad uv-version bisection is no longer the first step. If failures persist with the Git fix, obtain the exact Cargo/nextest command and features, package patches, and redacted edit/sync logs, distinguishing lock failures from frozen-sync failures. Do not request credential values or unredacted configuration dumps.

## Upstream Git/libcurl mechanism and workaround

In astral-sh/uv#22584, maintainer charliermarsh suggested a relationship to curl 8.23.0 and Git commit `76524c2604bceee48481909e7824494498c39a34` (comment ID 6192417103, October 15, 2026 at 12:47:36 UTC). Inspection of the linked upstream records establishes a concrete mechanism:

- curl 8.23.0 was published on October 14, 2026, the same date as uv 0.13.1. Release: https://github.com/curl/curl/releases/tag/curl-8_23_0. The reporter's downgrade result identifies 8.23 as the relevant curl threshold; exact installed package revisions were not provided.
- curl commit `7a6bd027d0de2b6857472d35da090ef60cc29f19`, dated September 21, changes `CURLINFO_EFFECTIVE_URL` to omit credentials. Source: https://github.com/curl/curl/commit/7a6bd027d0de2b6857472d35da090ef60cc29f19.
- Git commit `76524c2604bceee48481909e7824494498c39a34`, dated September 28 and titled “http: handle curl stripping creds from effective url,” explains that Git compared the requested URL with curl's effective URL as strings. With curl 8.23.0, removing credentials alone can look like a redirect; Git then updates its base URL, clears HTTP authentication, and asks for credentials again on subsequent requests. The commit explicitly describes a redirect warning followed by authentication failure. Source: https://github.com/git/git/commit/76524c2604bceee48481909e7824494498c39a34.
- The Git patch changes `update_url_from_redirect` to use `match_effective_url`, accepting either an exact match or a match after removing credentials from the requested URL. This prevents credential removal alone from being interpreted as a redirect.

This mechanism closely matches the reporter's authenticated URL, redirect warning, and subsequent username error with terminal prompting disabled. The reporter now confirms that downgrading curl below 8.23 resolves all the reported test failures, supporting a shared Git/libcurl cause across the listed scenarios. The internal failure sequence has not been independently traced in the Gentoo environment, and the flush error was not analyzed separately. The exact Git build, linked libcurl package revision, and inclusion of the Git fix or a downstream backport remain unknown. The earlier successful Ubuntu reproduction does not identify Git's linked libcurl version or patch status and remains a separate observation.

The established downstream workaround is the reporter's downgrade to `curl < 8.23`. For a lasting fix, check the libcurl actually used by Git's HTTP transport, not only the standalone curl command's version, and validate Git containing the cited fix or an equivalent backport with curl 8.23.0. The Git patch has not yet been reported tested by the reporter. Keep this downstream result distinct from the earlier local `not_reproducible` outcome.

## Release and implementation evidence

- `CHANGELOG.md` dates uv 0.13.1 to October 14, 2026. Release notes and the 0.13.0-to-0.13.1 commit/file comparison were reviewed before attributing behavior to an upgrade.
- astral-sh/uv#22464, merged October 11, changes checks for already-installed Git dependencies pinned to full commits. Its diff does not change the reported private-install test bodies or Git authentication.
- astral-sh/uv#22519, merged October 13, copies local repository fixtures into isolated test directories. Its diff does not alter the nine private-Git scenarios.
- astral-sh/uv#22452 caches compiled snapshot filters; astral-sh/uv#22453 avoids loading the Python catalog for contexts that request no Python. Their descriptions were reviewed as test-helper changes in the release. The reported tests request Python 3.12.
- In the source inspected during reproduction, `crates/uv/Cargo.toml` enables `test-git` through `test-defaults`. All nine reported tests require it; that source has no separate private-Git feature. Three of the install tests additionally exclude Windows. The maintainer subsequently announced the additional gate described above; its implementation has not been verified here.
- `crates/uv-git/src/git.rs::fetch_with_cli` invokes system Git and disables terminal prompting. This explains why Git's loss of HTTP authentication surfaces as a terminal-prompts-disabled failure in the uv tests; the upstream Git commit and reporter's downgrade result supply the compatibility evidence.
- `crates/uv-project-commands/src/sync.rs` uses `MemberDiscovery::Existing` for frozen sync, consistent with the final implementation of astral-sh/uv#19423. The original PR body describes an earlier implementation and should not be used as the current algorithm.

## Related

- astral-sh/uv#19419 (issue, closed) — Workspace-member source credentials were omitted under frozen sync. The reported frozen-sync test covers this historical bug. Its passing execution here does not support recurrence.
- astral-sh/uv#19423 (pull request, merged May 17, 2026) — Fixed astral-sh/uv#19419 and added the exact frozen-sync regression test. Historical context, not a confirmed fix for this report.
- astral-sh/uv#8467 (issue, open when previously inspected) — The same private-at-ref install test timed out after 90 seconds in October 2024. Different observed failure mode; no shared cause established.

## Search scope and excluded candidates

Earlier context gathering searched open and closed issues and PRs for the exact test identifiers, private fixture name, Git username and flush errors, uv 0.13.1, Gentoo, private-Git tests, and credentials. It also screened the 500 newest PR titles and inspected referenced fixes and discussions. PR keyword searches returned inconsistent results and some requests were rate-limited, so the negative duplicate result is qualified.

The earlier candidate exclusions remain appropriate: astral-sh/uv#15188 concerns snapshot path filtering; astral-sh/uv#17509 concerns Gentoo environment/Python discovery; astral-sh/uv#1980 and astral-sh/uv#16895 concern credential-helper side effects; astral-sh/uv#10493 concerns interactive publish prompting. astral-sh/uv#22357 is a URL-serialization refactor; astral-sh/uv#7453 and astral-sh/uv#8529 discuss older Git credential propagation or configuration. None establishes the present failure mechanism.

## Next maintainer action

Follow the reporter's planned handoff to Gentoo's Git maintainer. The reporter-verified workaround is `curl < 8.23`; the lasting remediation to validate is Git commit `76524c2604bceee48481909e7824494498c39a34` or an equivalent downstream backport, followed by the affected tests with curl 8.23.0. No Git package fix, downstream tracking reference, or issue closure is established by the supplied comment. Request further diagnostics only as needed to validate that fix or investigate remaining failures. Independently track the additional test-feature gate; its name, default behavior, and release availability remain unspecified. Preserve the bug classification and historical local reproduction result, while treating the downstream workaround as verified by the reporter.

No GitHub changes were made, and no checkout files were modified during reproduction. The pre-existing checkout status was left unchanged.
