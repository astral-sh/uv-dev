# Does uv git dependencies support shadow fetch --depth=1

Issue: astral-sh/uv#22957

Classification: duplicate

## Summary

astral-sh/uv#1737 tracks the same large-repository download concern. astral-sh/uv#8190 proposes lazy tree fetching and remains open; astral-sh/uv#3950 was an earlier unmerged attempt. uv 0.13.5 does not support configurable shallow Git dependency fetches.

The reporter asks whether Git dependencies can use shallow fetching with `--depth=1` to avoid lengthy downloads from a large repository. The report specifies macOS and uv 0.13.5, but includes no repository URL, exact uv command, timing, error, or claim that an earlier version behaved differently. “Shadow fetch” appears to refer to shallow fetching.

## Draft response

uv 0.13.5 does not support a `--depth=1` option for Git dependencies. Reducing downloads for large repositories is tracked in astral-sh/uv#1737. The proposed implementation in astral-sh/uv#8190 fetches trees and blobs lazily while retaining Git history needed by build tools such as setuptools-scm; it is still open. Please follow astral-sh/uv#1737 and add any repository-specific performance details there.

## Classification

Although phrased as a support question, the requested reduction in large Git dependency downloads is already tracked by open astral-sh/uv#1737 and its pre-existing implementation attempt astral-sh/uv#8190. uv 0.13.5 fetches without --depth or partial-clone filtering; the proposed optimization remains unmerged. No previously fixed behavior or regression is reported.

The canonical discussion covers reducing initial Git transfers for large repositories. Shallow history and partial fetching are different approaches: `--depth=1` truncates history, whereas the proposed `--filter=tree:0` optimization defers tree and blob downloads. The related pull request is evidence of ongoing work toward the same performance goal, not evidence that shallow fetching is supported.

## Related

- astral-sh/uv#1737 — Performance with large git repos (issue, open). Canonical discussion of slow Git dependency installation from large repositories, including a roughly 4 GB download and proposals to reduce initial fetch size. Maintainers link astral-sh/uv#3950 and astral-sh/uv#8190 as implementation attempts. This matches the reporter's goal of avoiding lengthy downloads.
- astral-sh/uv#8190 — Load Git trees lazily on checkout (pull request, open). Existing implementation attempt explicitly targeting astral-sh/uv#1737 and astral-sh/uv#3287. It proposes --filter=tree:0 to fetch trees and blobs on demand, retaining history for build tools such as setuptools-scm; it does not implement --depth=1. It remains unmerged.
- astral-sh/uv#3950 — Fetch git trees on demand (pull request, closed). Earlier attempt to reduce transfers for large or old repositories using --filter=tree:0, referenced by the canonical issue and astral-sh/uv#8190. Closed without merging after test failures and hangs, so it is historical context, not a released fix.

## Supporting evidence

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

## Handoff status

The response above is a draft for maintainer review. No issue comments, labels, pull requests, or other GitHub state were changed. No checkout files were modified. Only this temporary issue-context README was authored.
