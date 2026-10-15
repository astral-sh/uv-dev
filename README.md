# Multiple test regressions in 0.13.1

Issue: astral-sh/uv#22584

Classification: bug

## Summary

The reporter lists nine private-Git integration-test failures on Gentoo Linux amd64 with uv 0.13.1 and asks whether these tests should run for downstream users. Only `pip_install::install_git_private_https_multiple_pat` has detailed output: a success snapshot becomes uv exit 1 after Git fetch exits 128, with a redirect warning, `could not read Username for 'https://github.com': terminal prompts disabled`, and `expected flush after ref listing`.

**Reproduction outcome: not_reproducible.** All nine reported test scenarios passed against the published uv 0.13.1 binary on Ubuntu x86_64 with Git 2.55.0 and Python 3.12.3. The detailed multiple-PAT scenario also passed against the installed uv 0.13.0 binary. This does not rule out a Gentoo build, Git configuration, transport, or environment difference; no common root cause or uv-version regression has been established.

Maintainer charliermarsh confirmed that fixture credentials are intentionally shipped in the repository and that the private repositories test behavior rather than protect confidential contents (astral-sh/uv#22584, comment ID 6190830431, October 15, 2026 at 11:19:58 UTC). This confirms the intent behind the inspected tests supplying their own fixture credentials: downstream users are not expected to obtain separate private-repository access. In the inspected source, the tests are gated by the default-enabled `test-git` feature. The two rate-limit tests simulate GitHub API responses locally and do not demonstrate actual GitHub rate limiting. The reporter has not yet supplied the downstream details needed to explain the failures.

**Maintainer direction:** charliermarsh plans to gate these private-Git tests behind an additional feature beyond `test-git` (astral-sh/uv#22584, comment ID 6190687672, October 15, 2026 at 11:11:51 UTC). This is a stated implementation plan. The supplied discussion does not specify a feature name, default-enablement policy, implementation PR, or release target. The additional gate would give separate control over these tests; it does not establish why the authenticated fetches failed downstream.

**Upstream investigation lead:** the maintainer linked curl 8.23.0 and a Git fix for incorrectly treating credential removal from the effective URL as a redirect. The inspected Git commit describes discarding authentication and prompting again, closely matching the reported redirect warning and username failure. Applicability to the reporter remains unconfirmed; see the upstream evidence below.

## Classification

Retain the bug classification as a reported integration-test regression. The expected behavior is successful installation, dependency editing, and frozen synchronization, not failure merely because the fixtures are private. Current execution supports those success expectations and does not confirm the reported failures locally.

No confirmed duplicate was found. In particular, failure of a regression test associated with astral-sh/uv#19419 would not by itself establish recurrence of its historical workspace-discovery bug. The Git/libcurl interaction described below is established by upstream source, but its applicability to this report remains unconfirmed. Credential expiry and other downstream configuration causes also remain unconfirmed. The planned additional feature gate does not change the bug classification or the unsuccessful local reproduction outcome.

## Reproduction

### Environment and method

- Ubuntu 24.04.5 LTS, Linux x86_64; Git 2.55.0; system CPython 3.12.3.
- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.0/x86_64/uv`.
- Comparison executable: `/tmp/uv-22584-j5bpqa/tools/bin/uv`, installed from the published `uv==0.13.1` wheel using the installed uv. Both executables reported their expected versions and `x86_64-unknown-linux-gnu`.
- Temporary reproduction root: `/tmp/uv-22584-j5bpqa`. Environments, package caches, Cargo cache and build output, Python cache, configuration, and test directories were confined there.
- Copied the nine existing test bodies into a temporary Rust harness with their original setup, command arguments, feature gates, and snapshots. Each function body was compared with the `0.13.1` release tag and was identical. The harness uses the checkout's `uv-test` helpers and the installed uv binaries; it does not build or run a replacement uv binary.
- Built the harness with Rust 1.99.0 in the development test profile. Git system/global configuration and credential helpers were disabled for isolation. The original helpers supplied fixture authentication internally. The runner retained only test status and fixed error indicators, not credential-bearing command output.
- Tests were invoked explicitly, one named scenario per invocation, with fresh `TestContext` directories and package caches. The multiple-PAT case was run on both uv versions; the other eight cases were run on 0.13.1.

The reporter did not provide a last known-good version. Version 0.13.0 is a comparison version, not a confirmed downstream baseline. This harness does not recreate Gentoo's package recipe, downstream patches, full Cargo test invocation, or inherited Git configuration.

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

### Information needed to reproduce the downstream failure

Obtain the exact Cargo/nextest command and enabled features, Gentoo package recipe and patches, Git and libcurl versions, Python version, last passing uv version, and whether proxying or Git URL rewriting is involved. Request redacted full output from an edit test and the frozen-sync test, including whether the latter fails during `uv lock` or the subsequent frozen sync. A same-machine comparison of 0.13.0 and 0.13.1 using unchanged Git configuration would help isolate a version regression. Do not request credential values or unredacted configuration dumps.

## Upstream Git/libcurl investigation lead

In astral-sh/uv#22584, maintainer charliermarsh suggested a relationship to curl 8.23.0 and Git commit `76524c2604bceee48481909e7824494498c39a34` (comment ID 6192417103, October 15, 2026 at 12:47:36 UTC). Inspection of the linked upstream records establishes a concrete mechanism:

- curl 8.23.0 was published on October 14, 2026, the same date as uv 0.13.1. Release: https://github.com/curl/curl/releases/tag/curl-8_23_0. This timing is consistent with a dependency change coinciding with the reported uv upgrade, but does not establish the reporter's installed curl version.
- curl commit `7a6bd027d0de2b6857472d35da090ef60cc29f19`, dated September 21, changes `CURLINFO_EFFECTIVE_URL` to omit credentials. Source: https://github.com/curl/curl/commit/7a6bd027d0de2b6857472d35da090ef60cc29f19.
- Git commit `76524c2604bceee48481909e7824494498c39a34`, dated September 28 and titled “http: handle curl stripping creds from effective url,” explains that Git compared the requested URL with curl's effective URL as strings. With curl 8.23.0, removing credentials alone can look like a redirect; Git then updates its base URL, clears HTTP authentication, and asks for credentials again on subsequent requests. The commit explicitly describes a redirect warning followed by authentication failure. Source: https://github.com/git/git/commit/76524c2604bceee48481909e7824494498c39a34.
- The Git patch changes `update_url_from_redirect` to use `match_effective_url`, accepting either an exact match or a match after removing credentials from the requested URL. This prevents credential removal alone from being interpreted as a redirect.

This mechanism closely matches the reporter's authenticated URL, redirect warning, and subsequent username error with terminal prompting disabled. It is a stronger lead than a generic authentication failure, but it has not been reproduced in the reporter's environment and does not independently explain every listed failure or establish the cause of the flush error. The reporter's Git build, linked libcurl version, and inclusion of this fix or a downstream backport are unknown. The recorded successful Ubuntu reproduction does not identify Git's linked libcurl version or patch status, so it does not settle this comparison.

Prioritize checking the libcurl actually used by Git's HTTP transport, not only the standalone curl command's version, and whether the Git package contains the cited fix. If the affected combination is present, compare the same uv version and test environment with Git containing the fix or a downstream backport. This is a candidate remediation to validate, not an established workaround or a reason to change the reproduction outcome.

## Release and implementation evidence

- `CHANGELOG.md` dates uv 0.13.1 to October 14, 2026. Release notes and the 0.13.0-to-0.13.1 commit/file comparison were reviewed before attributing behavior to an upgrade.
- astral-sh/uv#22464, merged October 11, changes checks for already-installed Git dependencies pinned to full commits. Its diff does not change the reported private-install test bodies or Git authentication.
- astral-sh/uv#22519, merged October 13, copies local repository fixtures into isolated test directories. Its diff does not alter the nine private-Git scenarios.
- astral-sh/uv#22452 caches compiled snapshot filters; astral-sh/uv#22453 avoids loading the Python catalog for contexts that request no Python. Their descriptions were reviewed as test-helper changes in the release. The reported tests request Python 3.12.
- In the source inspected during reproduction, `crates/uv/Cargo.toml` enables `test-git` through `test-defaults`. All nine reported tests require it; that source has no separate private-Git feature. Three of the install tests additionally exclude Windows. The maintainer subsequently announced the additional gate described above; its implementation has not been verified here.
- `crates/uv-git/src/git.rs::fetch_with_cli` invokes system Git and disables terminal prompting. That explains the form of an unauthenticated-fetch error but does not identify why the reporter's authenticated fetch failed.
- `crates/uv-project-commands/src/sync.rs` uses `MemberDiscovery::Existing` for frozen sync, consistent with the final implementation of astral-sh/uv#19423. The original PR body describes an earlier implementation and should not be used as the current algorithm.

## Related

- astral-sh/uv#19419 (issue, closed) — Workspace-member source credentials were omitted under frozen sync. The reported frozen-sync test covers this historical bug. Its passing execution here does not support recurrence.
- astral-sh/uv#19423 (pull request, merged May 17, 2026) — Fixed astral-sh/uv#19419 and added the exact frozen-sync regression test. Historical context, not a confirmed fix for this report.
- astral-sh/uv#8467 (issue, open when previously inspected) — The same private-at-ref install test timed out after 90 seconds in October 2024. Different observed failure mode; no shared cause established.

## Search scope and excluded candidates

Earlier context gathering searched open and closed issues and PRs for the exact test identifiers, private fixture name, Git username and flush errors, uv 0.13.1, Gentoo, private-Git tests, and credentials. It also screened the 500 newest PR titles and inspected referenced fixes and discussions. PR keyword searches returned inconsistent results and some requests were rate-limited, so the negative duplicate result is qualified.

The earlier candidate exclusions remain appropriate: astral-sh/uv#15188 concerns snapshot path filtering; astral-sh/uv#17509 concerns Gentoo environment/Python discovery; astral-sh/uv#1980 and astral-sh/uv#16895 concern credential-helper side effects; astral-sh/uv#10493 concerns interactive publish prompting. astral-sh/uv#22357 is a URL-serialization refactor; astral-sh/uv#7453 and astral-sh/uv#8529 discuss older Git credential propagation or configuration. None establishes the present failure mechanism.

## Next maintainer action

First check the reporter's Git package, the libcurl used by its HTTP transport, and whether Git commit `76524c2604bceee48481909e7824494498c39a34` or an equivalent backport is present. Obtain the downstream details and redacted logs listed above; validate the upstream lead with uv held constant before attributing the failures to uv 0.13.1. Independently track the planned additional feature gate and confirm its name, default behavior, and release availability before recommending downstream feature flags. The existing successful reproduction and bug classification remain unchanged; neither the upstream lead nor the gating plan is a confirmed fix for this report.

No GitHub changes were made, and no checkout files were modified during reproduction. The pre-existing checkout status was left unchanged.
