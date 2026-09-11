# Support for `--no-cache-package` in `sync` and `run`

Issue: astral-sh/uv#21623

Classification: duplicate

## Summary

The reporter requests a package-scoped counterpart to `--no-cache` for `uv sync` and `uv run`, available from both the command line and `uv.toml`/`pyproject.toml`. The proposed option would avoid cache reads and writes for named packages, forcing them to be downloaded and built on every invocation while leaving caching enabled for other packages. The motivating cases are incomplete package metadata and builds whose output depends on environment variables or other external state.

The same underlying request is already tracked in astral-sh/uv#7642. Its second section asks for a way to mark a package so its compiled wheels are not cached when the build depends on the active HPC module or architecture. A maintainer explicitly identified `--no-cache-package <name>` as a possible interface in that discussion.

In the current discussion, a maintainer asked why the existing `--refresh-package <name>` option would not satisfy the request. They also clarified that `--no-cache` works by constructing an entirely new temporary cache directory and indicated that doing so per package is unlikely. The remaining question is whether package refresh provides the reporter's required behavior—particularly rebuilding on every invocation and avoiding cache writes—or whether the requested strict cache isolation is materially different.

## Maintainer feedback and open question

The latest maintainer feedback narrows the design question from “add a package-scoped cache flag” to “identify behavior that `--refresh-package <name>` does not already provide.” No final decision was made. Before prioritizing a new option, the discussion needs a concrete explanation or reproduction showing why package refresh is insufficient for the motivating package and build conditions.

## Draft response

This package-scoped cache opt-out is already tracked in astral-sh/uv#7642. The second request there covers compiled packages whose build output depends on external environment state, and the discussion specifically identifies `--no-cache-package <name>` as a possible interface. Let's centralize the feature discussion there.

For now, the closest supported workaround is:

```toml
[tool.uv]
reinstall-package = ["theseus-ai"]
```

That forces the named package to be reinstalled and implies a package refresh, but it does not provide the strict no-read/no-write cache behavior requested here.

## Classification

This is a duplicate because the open astral-sh/uv#7642 already tracks the same underlying capability: selectively disabling compiled-wheel caching for a named, environment-sensitive dependency. The earlier issue has a broader HPC title and also contains an unrelated virtual-environment-location request, but its second request and maintainer discussion match the behavior proposed here closely enough to centralize design and prioritization there.

The report would otherwise be an enhancement because it asks for a new CLI and configuration capability rather than identifying behavior that violates an existing contract. The existing `--refresh-package` and `reinstall-package` mechanisms are related but do not have the requested cache read/write semantics.

## Related

- astral-sh/uv#7642 — **Using uv on HPC Clusters** (open issue). Its second request asks for a per-package setting that prevents caching compiled wheels when build output varies with loaded MPI modules or architecture. A maintainer recommends `reinstall-package` as the current workaround and explicitly raises `--no-cache-package <name>` or `--no-cache-binary` as possible additions. This is the canonical duplicate.
- astral-sh/uv#7282 — **Make metadata caching of local projects opt-in when it is dynamic?** (open issue). This adjacent discussion covers dynamic local-project metadata and confirms `reinstall-package` as the existing way to mark a package for unconditional rebuild and reinstall. It does not request strict cache bypass or repeated download of an arbitrary registry dependency.
- astral-sh/uv#10170 — **Allow environment variables to be included in cache keys** (merged pull request). This added environment variables to local-project cache keys, addressing one trigger condition by invalidating a build when a declared variable changes. It does not select an arbitrary dependency for cache bypass and does not suppress cache writes.

## Search coverage and evidence

The report was decomposed into package-scoped cache bypass, support in `sync` and `run`, persistent TOML configuration, forced re-download/rebuild, incomplete metadata, and environment-sensitive build output. Searches covered the literal proposed flag and phrases such as “no cache,” “specific package,” “force re-download,” and “force rebuild,” plus conceptual alternatives including selective/per-package cache bypass, compiled-wheel caching, cache invalidation, dynamic metadata, `refresh-package`, `reinstall-package`, and environment-variable cache keys. Open and closed issues and open, closed, and merged pull requests were searched.

The comments and reference chains for the strongest candidates were inspected. astral-sh/uv#7642 is the only prior thread that states the same proposed flag and motivating behavior. astral-sh/uv#7282 and astral-sh/uv#10170 establish narrower rebuild and invalidation mechanisms. No pull request implementing package-scoped no-cache semantics was found. astral-sh/uv#10575 was plausible because it concerns cached environment-sensitive builds, but it tracks reuse across differing `--config-settings`, not a selective cache opt-out. astral-sh/uv#15700 was also ruled out because it concerns mutations propagating through hard-linked installed files, not build-cache selection. The astral-sh/uv#7642 reference to astral-sh/uv#1495 concerns centralized virtual-environment storage, the other half of that bundled report, and is unrelated to this request.
