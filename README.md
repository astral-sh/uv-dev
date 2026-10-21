# uv 0.13.5 listed in CHANGELOG but release appears to have been canceled

Issue: astral-sh/uv#22793

Classification: bug (release-status/documentation discrepancy; no runtime defect established)

## Summary

The reporter observes that the changelog at commit
`79422c835f214875852bf2b2cb950c44684eb046` lists uv 0.13.5 as released, while
GitHub/download availability stops at 0.13.4. They cite a canceled release workflow,
ask whether cancellation was intentional, and explain that automation deriving a
version from the changelog can select an unavailable download. No uv command,
project configuration, or platform-specific failure is reported; Python is unrelated.

Read-only checks on 2026-10-21, completed at approximately 14:05 UTC, reproduce the
reported state. The exact changelog says “Released on 2026-10-21”; GitHub's latest
published release is 0.13.4; the 0.13.5 release API and installer URL return HTTP
404; and the cited workflow concluded `cancelled`. The 0.13.4 installer URL returns
HTTP 200. These observations establish the publication mismatch, not the reason
for cancellation or whether canceling was incorrect.

astral-sh/uv#22787 prepared 0.13.5; astral-sh/uv#22788 subsequently reverted its
Python update. astral-sh/uv#22789 separately requests refreshed Docker images.
No duplicate or confirmed regression of a previously fixed bug was established.

## Classification

Retain the bug classification for the release-status/documentation discrepancy:
the reported changelog explicitly describes a release that was not available from
GitHub at observation time. This is also a release-process clarification request.
Reproducing the external state does not establish a defect in uv's runtime, an
incorrect cancellation, or an obligation to publish the prepared version.

`CONTRIBUTING.md` documents merging the changelog and version bump before manually
starting the release workflow. Repository metadata can therefore precede a
published release. Automation requiring downloadable versions should use published
release metadata, such as `https://api.github.com/repos/astral-sh/uv/releases/latest`,
rather than treating the development branch's changelog as a publication guarantee.
Whether the cancellation was intentional still requires maintainer confirmation.

## Reproduction

**Outcome: reproducible.** This is a release-publication observation, so no Python
project fixture or uv runtime regression test is needed. The report specifies
platform “any,” version 0.13.5, and Python “unrelated.” Checks ran on Linux x86_64
(kernel `7.0.0-1017-azure`) with the installed executable on PATH reporting
`uv 0.13.4 (x86_64-unknown-linux-gnu)` and Python 3.12.3. These local versions do
not determine the server-side release state.

Minimal read-only checks:

```sh
uv --version
reproduction_directory="$(mktemp -d /tmp/uv-22793-XXXXXX)"
cd "$reproduction_directory"
gh api 'repos/astral-sh/uv/contents/CHANGELOG.md?ref=79422c835f214875852bf2b2cb950c44684eb046' --jq .content | base64 --decode > CHANGELOG.md
sed -n '1,14p' CHANGELOG.md
gh api repos/astral-sh/uv/releases/latest --jq '{tag_name, published_at}'
gh api repos/astral-sh/uv/releases/tags/0.13.5 --jq .tag_name
gh api repos/astral-sh/uv/actions/runs/39535970462 --jq '{status, conclusion, head_sha, updated_at}'
curl -q --head --silent --show-error --location --connect-timeout 15 --max-time 45 --output /dev/null --write-out 'uv 0.13.5 installer: HTTP %{http_code}\n' https://github.com/astral-sh/uv/releases/download/0.13.5/uv-installer.sh
curl -q --head --silent --show-error --location --connect-timeout 15 --max-time 45 --output /dev/null --write-out 'uv 0.13.4 installer: HTTP %{http_code}\n' https://github.com/astral-sh/uv/releases/download/0.13.4/uv-installer.sh
```

Observed results:

- The exact changelog has `## 0.13.5` at line 6 and `Released on 2026-10-21.` at
  line 8. Its Python section advertises python-build-standalone 20261020.
- The latest-release API returns `tag_name: 0.13.4`, published at
  `2026-10-20T15:24:59Z`.
- The release-by-tag request for 0.13.5 exits 1 with `Not Found (HTTP 404)`.
- Workflow 39535970462 returns `status: completed`, `conclusion: cancelled`,
  head SHA `140db8caa82542e66d444a1a1162a41d8584ea6e`, and update time
  `2026-10-21T03:03:47Z`.
- The 0.13.5 installer HEAD request returns HTTP 404; the same asset for 0.13.4
  returns HTTP 200. No installer was executed and no installed uv was updated.

The fetched public changelog is retained at
`/tmp/uv-22793-pgiaxw45/CHANGELOG.md`. The reported commit's changelog was unavailable
through local `git show`, so its contents were retrieved through GitHub's contents
API at the exact SHA. No repository fetch or checkout modification was needed.
No project files or package caches were created. PyPI and other distribution
registries were not independently checked; download evidence concerns GitHub's
installer asset. Publication availability may change after these observations.

**Existing test coverage:** Searches under `crates/uv/tests/` and
`crates/uv-client/tests/it/` found no matching test of a changelog claiming a
published release after a canceled workflow. In
`crates/uv/tests/it/self_update.rs`, `check_self_update` installs/updates uv and
asserts that the resulting binary's `--version` command succeeds;
`test_self_update_uses_custom_path_with_ghe_override` supplies a mock successful
GitHub release response for version 9.9.9 and snapshots the dry-run update message.
Their setup and assertions do not cover changelog/publication consistency or the
reason for a live workflow cancellation. No repository tests were added or run.

The reported mismatch needs no further reproduction input. The unanswered question
is the maintainer's reason for cancellation; neither status metadata nor the
subsequent Python rollback proves intent or causation.

## Related

- astral-sh/uv#22787 — Bump version to 0.13.5 (merged pull request). Merged on
  2026-10-21 at 02:55:36 UTC, adding the version and changelog entry. Its merge SHA
  matches the canceled workflow's head SHA. This is release preparation, not a
  discussion resolving the mismatch.
- astral-sh/uv#22788 — Revert python-build-standalone release sync (merged pull
  request). Merged at 04:26:17 UTC, after cancellation. Its body reports
  intermittent SIGBUS crashes from the `cpython-3.14.8-linux-x86_64-gnu` archive
  during CodSpeed walltime profiling and restores metadata to 20261013. It does
  not explicitly explain the release cancellation or correct the changelog's
  publication status.
- astral-sh/uv#22789 — Rebuild 0.13.4 Docker images to pick up patched
  ca-certificates (eMudhra CA compromise) (open issue). Requests rebuilding
  0.13.4 images or releasing 0.13.5. It concerns the same release window but
  tracks stale Docker base images; it is not an established duplicate or evidence
  of why the release was canceled.

## Supporting evidence

- The exact reported changelog is available at
  https://github.com/astral-sh/uv/blob/79422c835f214875852bf2b2cb950c44684eb046/CHANGELOG.md.
  That commit is dated 11:50:25 UTC, after cancellation and the Python rollback;
  its release wording and Python-update description remained present.
- astral-sh/uv#22786 merged the Python update at 01:46:38 UTC on 2026-10-21.
  The version bump followed in astral-sh/uv#22787. Its body and inspected comments
  provide no cancellation explanation.
- Release workflow https://github.com/astral-sh/uv/actions/runs/39535970462 started
  at 02:58:22 UTC and was last updated at 03:03:47 UTC. Its jobs API shows that
  planning and the release gate succeeded; builds and `publish-github` were
  canceled; and PyPI and Docker publication were skipped. No returned job had a
  failure conclusion. The inspected recent release-run listing contained no
  later retry or replacement.
- The diff for astral-sh/uv#22788 restores Python metadata and sysconfig targets
  to 20261013 without changing the changelog or uv version. Its body and inspected
  comments do not explicitly connect the rollback to the workflow cancellation.
- `CONTRIBUTING.md` documents preparing and merging version/changelog changes
  before running the release workflow. `scripts/release.sh` generates those
  changes during preparation. `.github/workflows/release.yml` gates GitHub
  publication on prerequisite jobs and creates the release afterward. This
  explains how repository metadata can precede publication, without establishing
  the cancellation's cause.

## Search coverage and exclusions

Issue and PR searches covered 0.13.5, the reported commit SHA, workflow ID,
changelog, release canceled/cancelled, unpublished or unavailable releases,
automation selecting the latest release, and internal:releases. Release,
publish, and changelog changes and historical release failures were considered.
PR search omitted directly verified matches, so the investigation also inspected
the 200 most recently updated PRs, commit metadata, and relevant PR bodies,
comments, and diffs. Search results are not exhaustive; a temporary secondary
rate limit also affected one batch before a successful retry.

The Python update/revert chain was followed. Historical partial publication in
astral-sh/uv#3198 and its cargo-dist upgrade fix astral-sh/uv#3217 differs from this
canceled run, so it does not establish a regression. Other exclusions were
astral-sh/uv#1774 (missing release notes), astral-sh/uv#18813 (dry-run manifest
failure), astral-sh/uv#6440 (changelog generation for user projects),
astral-sh/uv#4432 (the 2024 PyPI size-limit incident), astral-sh/uv#15367 (missing
extended DockerHub images), and astral-sh/uv#16350 (Docker rate limits). None was
established as a duplicate.

## Maintainer follow-up

Confirm whether the release was intentionally canceled and document the reason.
Clarify the changelog's release status and Python-update description against the
actual release plan. No publication date or restart is established by the
inspected evidence. Recommend published-release metadata for automation that
requires an available download.

No GitHub changes were made and no checkout files were modified. Only this
issue-context README and a temporary copy of the public changelog were written.
