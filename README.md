# proxy-index: path-independent artifact mapping (Artifactory layout), follow-up to astral-sh/uv#20790

Issue: astral-sh/uv#23275
URL: https://github.com/astral-sh/uv/issues/23275
Classification: duplicate

## Summary

The reporter requests an opt-in, per-proxy mode that installs locked artifacts from a proxy whose directory layout differs from the upstream registry. Their Artifactory Simple API advertises the same filename and SHA-256 as PyPI, but under a package/version path rather than PyPI's hashed directories. They report that requests using PyPI's suffix under Artifactory return HTTP 404.

The proposed mode would query the proxy's package listing, match the locked filename, and verify the locked hash. It would incur additional index requests and remain disabled by default. The reporter separately asks how creating or updating a lock through such a proxy could obtain genuine upstream URLs.

The reporter has not run the new proxy-index feature against main. The expected invalid upstream URLs are nevertheless supported by the implementation, documentation, and an earlier GAR reproduction. The quoted roughly five-second cost for 235 packages is an earlier contributor's GAR measurement, not an Artifactory benchmark or independently verified performance guarantee.

astral-sh/uv#6349 already contains the path-independent design discussion and Artifactory examples. astral-sh/uv#22231 closely overlaps installs from an unchanged lock. astral-sh/uv#20790 merged on November 17, 2026, with prefix mapping and deferred filename lookup.

## Draft response

The filename-and-hash lookup you propose is already discussed in astral-sh/uv#6349, including registries with different artifact layouts. Let's keep that discussion together there.

astral-sh/uv#20790 merged with prefix-only routing and deferred filename lookup. The implementation does not validate the derived upstream URLs, so a lock can work through the proxy while failing against upstream. Matching hashes do not make the paths interchangeable.

Please continue in astral-sh/uv#6349 with whether your environment can reach the upstream Simple API when creating or updating a lock. That matters for obtaining canonical URLs: looking up files in Artifactory during installation alone does not solve the locking side. The merged implementation has no opt-in filename lookup mode.

## Classification

astral-sh/uv#6349 already tracks the same underlying capability, including Artifactory/Nexus path differences and a maintainer's explicit filename-and-hash lookup design. astral-sh/uv#22231 independently covers the install-only case. Duplicate takes precedence despite the source-confirmed risk of writing invalid upstream URLs. astral-sh/uv#20790 deliberately deferred this capability rather than fixing it, so this is not a regression or a duplicate merely of an implementing PR.

The unsafe portability outcome is established by source and the prior GAR report; documenting the limitation does not make fabricated upstream URLs correct. Duplicate classification takes precedence because the underlying request and the relevant design questions already have an open canonical discussion. A fresh Artifactory execution is not necessary to establish that the current prefix transformation cannot reconstruct PyPI's different directory suffix.

No evidence establishes that a previously supported filename lookup mode was removed or that a released fix regressed. The earlier maintainer statement about a follow-up is evidence of deferred scope, not proof that another implementation exists or a commitment to a delivery date.

## Related

- astral-sh/uv#6349 — Request for `uv.lock` to support different index urls across different developer machines and CI environments (issue, open). Canonical discussion of portable lockfiles across proxies with different artifact layouts. Comments explicitly cover Nexus/Artifactory path differences; a maintainer describes resolving locked filenames through the proxy and checking hashes, while obtaining canonical URLs separately when locking. This already tracks the underlying request.
- astral-sh/uv#20790 — Support proxy indexes through reversible artifact URLs (pull request, merged). Introduced the current prefix-only implementation, merged November 17, 2026. The GAR report demonstrates fabricated upstream URLs when paths differ, and a maintainer explicitly defers filename lookup to a follow-up. Its description and the checkout confirm that upstream URLs are not validated; this PR did not implement the requested fallback.
- astral-sh/uv#22231 — Allow index override when using uv sync --frozen (issue, open). Closely overlaps the install-only capability: install unchanged locked versions and hashes from a GitLab registry whose artifact paths differ from PyPI. It proposes index overrides and upstream fallback, whereas this report requests an opt-in per-proxy mode and separately asks about locking.

## Supporting evidence

- In astral-sh/uv#6349, a Nexus example already contrasts package/version URLs with PyPI's hashed paths: https://github.com/astral-sh/uv/issues/6349#issuecomment-2685799932. A later Artifactory migration report describes incompatible directory layouts and requests rediscovering artifacts from the registry while checking stored hashes: https://github.com/astral-sh/uv/issues/6349#issuecomment-3237850699.
- A maintainer explicitly states in astral-sh/uv#6349 that filenames and hashes can match while wheels are hosted anywhere: https://github.com/astral-sh/uv/issues/6349#issuecomment-4708310424. The same maintainer explains discarding all but the filename during proxied installation and querying the canonical index when writing a lock: https://github.com/astral-sh/uv/issues/6349#issuecomment-4710446931. These describe a proposed design, not the behavior eventually merged.
- The GAR reproduction on astral-sh/uv#20790 reports that prefix substitution creates nonexistent upstream URLs, while reverse substitution can still install through the proxy: https://github.com/astral-sh/uv/pull/20790#issuecomment-5182587338. The maintainer explicitly defers the differing-layout capability: https://github.com/astral-sh/uv/pull/20790#issuecomment-5373154189.
- In the checkout, docs/concepts/indexes.md:93 states that installing an existing lock rewrites URLs without querying the proxy's index. Lines 118–123 require identical suffixes and explicitly state that upstream paths are not checked during locking.
- In crates/uv-distribution-types/src/proxy_index.rs:194, ArtifactUrlMapping maps in both directions through rewrite. That function retains the suffix and changes the configured prefix without an upstream HTTP check. ProxyRoute::artifact_url_for_request at line 277 uses this mapping for downloads; IndexRoute::canonicalize_file at line 317 selects proxy canonicalization for resolver files.
- Prefix substitution can be mechanically reversible through the same proxy even when the recorded upstream URL is invalid. An existing PyPI lock routed to Artifactory can instead fail at the proxy because its hashed suffix was never advertised there. These are two directions of the same layout mismatch.
- The description of astral-sh/uv#22231 explicitly requires locked name/version/hash matching against a registry with paths unlike PyPI, without rebuilding or modifying the lock.

## Search coverage and exclusions

Decomposed the report into changed artifact paths/fabricated upstream URLs, opt-in filename-and-hash lookup during locked installs, and canonical URL discovery during locking. Used authenticated gh searches across open/closed issues and open/closed/merged PRs for proxy-index, artifact-base-url, proxy-for, filename-based, Artifactory, Google Artifact Registry, artifact paths, mirror paths, portable lockfiles, different URL schemes, canonical URLs, index overrides, filename/hash lookup, and closed 404/lock fixes. PR search returned no indexed matches; supplemented it with recently updated PR listings and direct inspection of referenced PRs and comments. Followed astral-sh/uv#20790 to astral-sh/uv#6349 and inspected astral-sh/uv#22231. Ruled out astral-sh/uv#23116 as a filename-lookup implementation: it addresses speculative prefetch and candidate error handling. Also inspected astral-sh/uv#11782 (closed, unmerged prefix-based predecessor), astral-sh/uv#22794 (metadata request preparation), astral-sh/uv#19625 (ordinary frozen URL semantics), and astral-sh/uv#8262 and astral-sh/uv#15741 (index selection/configuration). None establishes a fix or regression of path-independent lookup.

The strongest excluded follow-up candidate is astral-sh/uv#23116, Suggestions for proxy-indexes (open). Its description concerns speculative prefetch failures and errors from unused or alternate candidates; its comment does not identify filename lookup. It therefore cannot be cited as implementing the capability here.

astral-sh/uv#11782 is historical design context, not a shipped fix: it closed unmerged. The merged astral-sh/uv#22794 prepares wheel metadata requests for proxy routing and does not resolve filenames through a proxy's Simple API. The closed astral-sh/uv#19625 explains ordinary frozen installs using locked URLs, rather than the creation of nonexistent canonical URLs by proxy mapping. The index precedence and configuration reports in astral-sh/uv#8262 and astral-sh/uv#15741 point back to astral-sh/uv#6349.

## Maintainer next step

On November 19, 2026, maintainer zsol confirmed in astral-sh/uv#23275 that `main` did not yet support path-independent artifact lookup and said they planned to start work on the follow-up soon (https://github.com/astral-sh/uv/issues/23275#issuecomment-7013826872). The comment does not identify an implementation branch or release target, or say whether the follow-up will also handle upstream URL discovery during lock creation.

Centralize the Artifactory case in astral-sh/uv#6349 and retain the distinction between installation-time filename lookup and upstream URL discovery during lock creation. The useful missing deployment detail is whether the upstream Simple API is reachable while creating or updating locks.

Only the temporary issue-context README and supporting temporary handoff files were written. No checkout files or GitHub state were changed. No builds or runtime reproduction were performed.
