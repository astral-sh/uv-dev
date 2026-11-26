# proxy indexes - option to resolve dependencies with one index and download from another

Issue: astral-sh/uv#23412

Classification: enhancement

## Summary

The reporter must use an internal registry for package downloads. They want uv to resolve against upstream PyPI, where the upload timestamps needed by `exclude-newer` are available, and then fetch the selected artifacts from the internal registry. They also report that resolving through the registry is slow. They did not identify a uv version, command, registry implementation, or cause of the slowdown.

The proxy-index preview supports a related two-step workflow, not split routing during fresh resolution. The portable-lockfile design and artifact-path follow-up are the closest prior work; the timestamp reports explain the motivation but request different behavior.

## Draft response

The proxy-index preview introduced in uv 0.13.10 uses the proxy for both package lookups and downloads. It records upstream URLs in the lockfile, but it doesn't use PyPI's upload timestamps when resolving through the proxy. Resolving against PyPI while routing downloads through your registry would be a new mode.

If your policy allows a separate `uv lock` against PyPI without the proxy, you can then configure the proxy for `uv sync --frozen`. That routes locked artifacts through the proxy if their paths match after the configured prefixes; it doesn't cover resolution of isolated build requirements. Would that two-step workflow fit, or do you need both routes in a command that resolves and installs together?

## Classification

This is an enhancement: it asks for independent routing of resolution requests and artifact downloads within the same configuration. The current proxy intentionally replaces the package lookup endpoint and the artifact prefix together. A lockfile recording PyPI URLs does not imply that resolution queried PyPI. The report does not establish a uv regression or correctness defect, and the previously tracked workflows do not cover the requested mode.

## Related

- astral-sh/uv#20790 (merged PR) introduced proxy indexes in preview. Fresh resolution and artifact downloads use the proxy while the lockfile retains upstream URLs; frozen installs can rewrite locked artifact URLs without querying the proxy's Simple API.
- astral-sh/uv#23275 (open issue) tracks proxies whose artifact paths cannot be mapped by replacing a prefix. It addresses artifact lookup and canonical lock URLs for those layouts, not using upstream metadata instead of proxy metadata for fresh resolution.
- astral-sh/uv#6349 (closed issue) is the canonical portable-lockfile discussion. Maintainers closed it after the preview landed and directed the remaining nonmatching-path cases to astral-sh/uv#23275. An earlier design comment discussed upstream validation of proxy-selected results, not upstream-driven selection.
- astral-sh/uv#22231 (closed issue) asked to download from a different registry while syncing an existing lock. Maintainers pointed it to astral-sh/uv#6349 and astral-sh/uv#23275. It did not ask to separate metadata from downloads during fresh resolution.
- astral-sh/uv#21449 (open issue) confirms that missing upload timestamps on a pull-through cache can make candidates unavailable under `exclude-newer`. Its concrete failure is isolated build-dependency resolution in frozen CI, not a request for split index routing.

## Supporting evidence

- `CHANGELOG.md:6` and the GitHub release record date uv 0.13.10 to November 24, 2026. `docs/concepts/indexes.md:55` documents proxy package lookups, prefix-based download routing, and the frozen-install behavior. `crates/uv-client/src/registry_client.rs:436` passes the effective index URL to the Simple API client. `crates/uv/tests/it/proxy_index.rs:713` covers creating a lock with canonical URLs without upstream requests.
- `docs/concepts/resolution.md:819` documents the `upload-time` requirement. An opt-out skips the cutoff; it does not restore the reporter's intended age check. The upstream-lock workaround is conditional: locking can require fetching an artifact for metadata or a build, and `--frozen` does not itself lock isolated build requirements, as explained in astral-sh/uv#21449.

## Search scope

Searched open and closed uv issues and open, closed, and merged pull requests for `proxy indexes`, `proxy-for`, `artifact-base-url`, `resolve = true`, and variants of resolving from one index and downloading from another. Conceptual searches covered upstream metadata, mirror and frozen-lock downloads, artifact-path mapping, missing `upload-time` with `exclude-newer`, and slow registry resolution. Cross-checked PR searches against repository pull listings, linked changes, and release notes, and followed maintainer references to the portable-lockfile, path-mapping, and timestamp discussions.

The timestamp-only request in astral-sh/uv#12449 would make exceptions for missing dates, not keep the cutoff while routing downloads separately. The similarly titled astral-sh/uv#23116 concerns refactoring and prefetch candidate errors, not split routing. The slowdown in astral-sh/uv#16111 was tied to missing wheel metadata and range-request support on a particular registry; the new report does not establish the same mechanism.
