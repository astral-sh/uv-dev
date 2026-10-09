# Allow index override when using uv sync --frozen

Issue: astral-sh/uv#22231

Classification: duplicate

Reviewed: 2026-10-09

## Summary

astral-sh/uv#6349 is the canonical discussion. astral-sh/uv#19625 and astral-sh/uv#21747 confirm current frozen behavior. Open astral-sh/uv#20790 addresses proxy installation but has a path-layout limitation; the other items provide historical context.

The reporter creates and commits `uv.lock` on an internet-connected machine, then mirrors the project and selected Python artifacts into an air-gapped network. The GitLab registry uses artifact paths unrelated to the PyPI URLs in the lockfile. Only dependencies for the selected Python version and installation are mirrored, so resolving the entire project again is not viable.

They request that `UV_INDEX` or `--index` override artifact download locations during `uv sync --frozen` and the automatic synchronization performed by `uv run --frozen`. Matching must preserve the locked package name, version, and hash. The configured override should take priority, with fallback to the locked URL when the artifact is unavailable, and the lockfile must remain unchanged. Named-index authentication is part of the intended workflow, but no authentication failure is reported.

## Draft response

This use case is tracked in astral-sh/uv#6349: installing locked packages through a different mirror while preserving the lockfile and its hashes. Currently, `--frozen` uses the artifact URLs recorded in `uv.lock`; `UV_INDEX` and `--index` do not redirect those downloads, as confirmed in astral-sh/uv#19625.

astral-sh/uv#20790 is open work on proxy indexes, but its current design requires matching artifact paths after the configured base URLs. Your GitLab mirror's different paths are an important limitation, so that PR should not be treated as a complete solution for this setup.

Please add your partial-mirror, no-relocking, and fallback requirements to astral-sh/uv#6349 so we can keep the design discussion together.

## Classification

astral-sh/uv#6349 already tracks installing from alternative mirrors without changing the authoritative lockfile, including differing artifact URLs and hash validation. The frozen sync/run commands, partial GitLab mirror, and proposed fallback refine that existing request. Maintainer comments and the source confirm current frozen installs use locked artifact URLs. No previously shipped implementation of the requested capability or regression was established; the duplicate classification takes precedence over enhancement.

Centralize the discussion in astral-sh/uv#6349. Its open enhancement/needs-design discussion already includes installing through mirrors without contacting the original index and preserving canonical lockfile information. The proposed flag behavior and partial-mirror example add design requirements to that existing capability.

The duplicate recommendation is based on that pre-existing issue, rather than on a pull request created in response to this report. astral-sh/uv#20790 was opened on 2026-07-29, before this report on 2026-10-06, and remains open. astral-sh/uv#11782 closed without merging. The report supplies no uv version, exact error, or assertion that the requested capability previously worked.

## Related

- astral-sh/uv#6349 — Issue, open: Request for `uv.lock` to support different index urls across different developer machines and CI environments. Canonical discussion for using an unchanged lockfile across different package mirrors. Maintainers explicitly discuss installing through a proxy with locked versions and hashes, including different artifact paths. The partial, air-gapped GitLab mirror is a specific instance of this request.
- astral-sh/uv#19625 — Issue, closed: `uv sync --frozen` ignores all index/mirror configuration and uses hardcoded URLs from `uv.lock`. Reports the same frozen-sync behavior and proposes matching package/version/hash at an override index before falling back to locked URLs. Closed with maintainer confirmation that frozen installs intentionally use locked URLs; it also points to astral-sh/uv#6349.
- astral-sh/uv#20790 — Pull request, open: Support proxy indexes through reversible artifact URLs. Existing implementation work preserves canonical lockfile URLs and hashes while downloading through a proxy, including frozen installs. Its current design preserves paths after configured artifact bases; maintainer comments defer support for different path layouts to follow-up work. It therefore does not establish support for the reported GitLab layout or requested fallback.
- astral-sh/uv#21747 — Issue, closed: frozen block UV_DEFAULT_INDEX and UV_INDEX_STRATEGY. Recent report of frozen sync ignoring a configured mirror. A maintainer explains that downloads use locations in uv.lock and points to astral-sh/uv#20790 for potential support when the lockfile must stay fixed.
- astral-sh/uv#11782 — Pull request, closed: Support proxy index urls. Earlier proposal for CLI/environment proxy overrides across sync, run, lock, and add while retaining canonical URLs in the lockfile. Explicitly targeted astral-sh/uv#6349, but closed without merging; it is historical design work, not evidence of a shipped fix or regression.
- astral-sh/uv#13094 — Issue, closed: uv for air-gapped machines: uv should not record package url in uv.lock. Same requirement to move an internet-generated lockfile into an air-gapped environment without rewriting it, relying on hashes for artifact identity. Uses a local wheel directory and proposes omitting URLs, unlike this report's GitLab index override; maintainers declined omitting locked URLs.

## Supporting evidence

- In astral-sh/uv#19625, Charlie Marsh states: “This is intentional. `--frozen` installs from the lockfile, and the lockfile includes locked URLs.” This directly establishes the current contract for the reported download behavior.
- In astral-sh/uv#6349, Zsol describes preserving canonical URLs and authoritative versions/hashes while downloading through a proxy. Further comments explicitly discuss matching filenames and hashes despite different storage paths, and avoiding requests to the original index during installation. This is the closest discussion of the requested capability.
- The current description of astral-sh/uv#20790 narrows implementation to reversible URL mappings that preserve the path after an artifact base URL. A contributor reports that registries with different layouts cannot use that mapping; Zsol replies that this limitation will be addressed in follow-up work. That statement is a design direction, not evidence of a merged implementation or a release commitment. Do not present the open PR as sufficient for arbitrary GitLab paths or promise the requested fallback behavior.
- In astral-sh/uv#21747, a maintainer reiterates that frozen downloads use locked locations and links astral-sh/uv#20790 when the lockfile must remain fixed.
- `docs/concepts/projects/sync.md:9` documents automatic locking/syncing for `uv run`; line 22 documents `--frozen` using the lockfile without checking freshness.
- `crates/uv-lock-operations/src/lock.rs:167` returns the existing frozen lock without resolving the project. `crates/uv-project-commands/src/sync.rs:423` and `crates/uv-project-commands/src/run.rs:755` select frozen locking; the latter then calls `sync_from_lock`.
- `crates/uv-lock/src/lock/mod.rs:6718` materializes installation distributions from locked package data. Its `to_registry_wheel` implementation at line 9003 uses the stored wheel URL directly as an absolute file location. This supports the maintainer explanation for registry wheel downloads.

## Search scope and alternatives

Used authenticated gh to search open and closed issues separately for frozen/index behavior, UV_INDEX overrides, --index with --frozen, uv run with mirrors, air-gapped installation, lockfile mirrors, hash preservation, fallback, and avoiding re-resolution. Followed reporter links and maintainer references. Attempted all-state and merged-PR searches for frozen/index, proxy, mirror/lock, and the canonical issue; REST searches hit rate limits and GraphQL PR searches returned issue-only results. Supplemented these with title filtering of 500 recent PRs across open/closed/merged states and 500 open PRs, and direct inspection of referenced PRs and comments. No applicable merged fix was established. Ruled out astral-sh/uv#16996 (resolution-option diagnostics), astral-sh/uv#6950 (Python interpreter mirrors), and astral-sh/uv#22351 (loading unused build indexes). Also inspected astral-sh/uv#13587, astral-sh/uv#15519, and astral-sh/uv#15015; these provide broader air-gap, wheelhouse, or proxy context rather than a closer canonical discussion.

Searches separated the observable frozen-download behavior from the proposed proxy design. Literal terms included `UV_INDEX`, `--index`, `--frozen`, `uv run`, and `fallback`; conceptual terms included mirrors, proxies, air-gapped installs, lockfile portability, hashes, and re-resolution. Closed-issue and merged-PR searches checked for historical fixes.

The reporter-linked astral-sh/uv#16996 concerns silently ignored resolution options such as resolution strategy and prerelease selection, not installation-source substitution. astral-sh/uv#15519 additionally involves dev-group resolution and a local wheelhouse with conflicting `--no-index --frozen` options. astral-sh/uv#13587 is broader deployment guidance; its linked merged astral-sh/uv#6950 handles Python interpreter archives rather than project dependencies. The recent astral-sh/uv#22351 avoids reading unused build indexes during wheel-only installation and does not redirect locked downloads. astral-sh/uv#15015 explicitly points back to astral-sh/uv#6349.

## Verification and limits

This handoff is based on repository source, documentation, issue discussions, and directly verified pull-request states. No runtime reproduction was performed. Broad PR search was limited by the API behavior described above; direct inspection and bounded PR listings supplied the relevant implementation history. No evidence establishes a shipped fix that subsequently regressed.

Only the issue-context handoff and its structured result were written under the runner temporary directory. No checkout files were edited and no changes were made on GitHub.
