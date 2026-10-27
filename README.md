# Does uv git dependencies support shadow fetch --depth=1

Issue: astral-sh/uv#22957

Classification: duplicate

## Summary

astral-sh/uv#1737 tracks the same large-repository download concern. A maintainer now identifies astral-sh/uv#22854 as a potential solution: it proposes partial Git fetches behind `--preview-features git-partial-fetches` and remains open and unmerged. This retains commit history while reducing historical file downloads; it does not add `--depth=1`. astral-sh/uv#8190 and astral-sh/uv#3950 provide earlier implementation context. uv 0.13.5 does not support configurable shallow Git dependency fetches.

The reporter asks whether Git dependencies can use shallow fetching with `--depth=1` to avoid lengthy downloads from a large repository. The report specifies macOS and uv 0.13.5, but includes no repository URL, exact uv command, timing, error, or claim that an earlier version behaved differently. “Shadow fetch” appears to refer to shallow fetching.

## Maintainer guidance and implementation status

In the issue discussion, maintainer zanieb points to astral-sh/uv#22854 as something that might achieve the reporter's goal (October 27, 2026, comment ID 6446001914). This is a relevant implementation lead, not confirmation that it resolves the reporter's particular workload. No repository, timing, or reproduction details were added.

astral-sh/uv#22854 — Add partial Git fetches behind a preview feature — was created on October 22, 2026, before this issue. Its current diff introduces `git-partial-fetches` and uses `--filter=tree:0` to retain commit history while fetching trees and blobs for the requested revision. The PR is open with no merge date; the preview flag is proposed behavior, not an available workaround established for uv 0.13.5.

The PR description reports substantial improvements for repositories with unused history, but extra requests can make small cold checkouts slower. Those benchmarks were reported by the PR author and were not independently reproduced for this issue. The proposal uses a separate partial-fetch cache, so switching modes can require a fresh download. Source URLs containing credentials or query parameters, and configurations with an `--allow-insecure-host` exception, fall back to full fetches. Offline access to uncached history can fail; offline reconstruction of LFS checkouts and submodule handling have additional limitations described in the PR. The reporter has not supplied enough configuration to determine whether these conditions apply.

## Classification

Although phrased as a support question, the requested reduction in large Git dependency downloads is already tracked by open astral-sh/uv#1737 and its pre-existing implementation attempt astral-sh/uv#8190. uv 0.13.5 fetches without --depth or partial-clone filtering; the proposed optimization remains unmerged. No previously fixed behavior or regression is reported. The maintainer-linked astral-sh/uv#22854 adds a more direct implementation lead and predates this report; it does not change the duplicate classification or establish a released fix.

The canonical discussion covers reducing initial Git transfers for large repositories. Shallow history and partial fetching are different approaches: `--depth=1` truncates history, whereas the proposed `--filter=tree:0` optimization defers tree and blob downloads. The related pull request is evidence of ongoing work toward the same performance goal, not evidence that shallow fetching is supported.

## Related

- astral-sh/uv#22854 — Add partial Git fetches behind a preview feature (pull request, open). Explicitly suggested by a maintainer in astral-sh/uv#22957 as potentially meeting the download-performance goal. Its diff gates `--filter=tree:0` fetches behind `--preview-features git-partial-fetches`, retaining history rather than imposing shallow depth. It predates this issue and remains unmerged.
- astral-sh/uv#1737 — Performance with large git repos (issue, open). Canonical discussion of slow Git dependency installation from large repositories, including a roughly 4 GB download and proposals to reduce initial fetch size. Maintainers link astral-sh/uv#3950 and astral-sh/uv#8190 as implementation attempts. This matches the reporter's goal of avoiding lengthy downloads.
- astral-sh/uv#8190 — Load Git trees lazily on checkout (pull request, open). Existing implementation attempt explicitly targeting astral-sh/uv#1737 and astral-sh/uv#3287. It proposes --filter=tree:0 to fetch trees and blobs on demand, retaining history for build tools such as setuptools-scm; it does not implement --depth=1. It remains unmerged.
- astral-sh/uv#3950 — Fetch git trees on demand (pull request, closed). Earlier attempt to reduce transfers for large or old repositories using --filter=tree:0, referenced by the canonical issue and astral-sh/uv#8190. Closed without merging after test failures and hangs, so it is historical context, not a released fix.

## Supporting evidence

- The supplied full issue discussion identifies zanieb as a repository member and records the suggestion of astral-sh/uv#22854. GitHub PR metadata confirms that the PR is open, was created October 22, 2026, and has no merge date. Its diff adds `PreviewFeature::GitPartialFetches`, defines `PARTIAL_CLONE_FILTER` as `tree:0`, and conditionally applies that filter during Git fetches. This confirms the proposed mechanism, while its effectiveness for this reporter remains unverified.
- In both the checkout and the uv 0.13.5 tag, `crates/uv-git/src/git.rs:745` defines `fetch_with_cli`. It constructs `git fetch` with optional `--tags`, then `--force`, `--update-head-ok`, the remote URL, and refspecs. It adds neither `--depth` nor `--filter`. The requested ref can narrow which branches or tags are fetched, but that does not limit history depth.
- The checkout's `crates/uv-workspace/src/pyproject.rs:1214` defines `Source`; its Git variant exposes the URL, revision/branch/tag, subdirectory/path, LFS, and dependency selectors. It has no shallow or depth setting. Searches of the Git implementation, settings, schema, and dependency documentation found no shallow-fetch configuration.
- In astral-sh/uv#1737, maintainer comments dated March 18 and June 6, 2025 identify the two partial-fetch attempts. The June 6 explanation describes a shared Git database with local clones for individual revisions. An April 8, 2026 update points to renewed work on astral-sh/uv#8190.
- astral-sh/uv#8190 explicitly targets astral-sh/uv#1737 and astral-sh/uv#3287. Its diff adds `--filter=tree:0` and explains that build tools such as setuptools-scm may need Git history. Its body reports improved cold-cache performance and smaller caches in sample benchmarks, with tradeoffs for warm caches and multiple checkouts. The PR remains open with no merge date, so its behavior is not available through that PR in a released version.
- astral-sh/uv#3950 proposed the same tree filter in May 2024. Discussion records CI failures and a local test hang. A maintainer closed it as stale on January 15, 2025; it has no merge date.

The tagged source was read through the GitHub contents API at `ref=0.13.5`. No reproduction or benchmark was run: the report provides no target repository, and support status is established directly from the fetch implementation and configuration.

## Other candidates considered

- astral-sh/uv#3287 — [perf] uv pip install resolution is slow when installing from VCS (open). It contains explicit discussion of shallow and blobless clones, including the history requirements of setuptools-scm. A maintainer clarified that it remains open for selectively retrieving metadata such as `pyproject.toml`; the discussion directs general clone performance to astral-sh/uv#1737. It is less specific to this report's download concern.
- astral-sh/uv#1620 — Allow sparse checkouts/path exclusion for git dependencies (open, enhancement, needs-design). It concerns excluding large files or directories from a checkout, a separate capability from limiting history depth.
- astral-sh/uv#10130 — Is it possible to skip fetching git submodules for some repositories? (open, area:configuration, needs-decision). It mentions a shallow-clone flag as an additional idea, but its primary request is submodule selection and avoidance. The new report does not mention submodules.

## Search scope

Separated shallow-fetch support (--depth=1) from slow downloads for large Git dependencies. Searched open and closed issues using shadow fetch, shallow, depth=1, git slow, git large, large git, large repo, partial clone, blobless, and git history. Searched open, closed, and merged PRs for shallow, depth=1, clone, git fetch, partial clone, Git trees, and tree, including an explicit merged-Git search; PR searches returned no hits, so implementation candidates were followed from issue comments and inspected directly. Checked uv 0.13.5 and checkout fetch code plus source configuration. Inspected but ruled out as canonical matches astral-sh/uv#3287 (selective metadata retrieval during resolution), astral-sh/uv#1620 (sparse paths), and astral-sh/uv#10130 (submodule controls, with shallow cloning mentioned secondarily). No released shallow-fetch fix or regression was established.

Literal capability searches and download-performance searches were kept separate from searches for potential implementations. macOS and uv 0.13.5 were treated as report context rather than search restrictions. Candidate bodies, comments, and PR reviews were inspected, and maintainer references were followed to establish the canonical discussion and implementation status.

The new maintainer reference was followed to astral-sh/uv#22854; its body, discussion, review metadata, and relevant diff were inspected. The earlier searches missed this implementation lead. The existing structured triage findings remain historical inputs; this README incorporates the additional evidence.

## Handoff status

Only this temporary issue-context README was updated. No public response was drafted or posted during this context update, and no GitHub state or checkout files were changed. The prior public-response draft was replaced with internal implementation status. The duplicate classification, original report details, related historical work, and lack of a reproduction remain unchanged.
