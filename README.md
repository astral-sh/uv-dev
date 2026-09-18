# granular and user-friendly management of cached packages

Issue: astral-sh/uv#21829

Classification: duplicate

## Summary

The reporter wants package-and-version-level cache management so that old versions can be found,
measured, and deleted without purging useful artifacts and downloading them again. The requested
operations are an inventory of cached package/version combinations, selective removal of one such
combination, and a per-combination disk-usage breakdown.

The closest open discussions collectively cover these capabilities. astral-sh/uv#1655 requests a
cache listing containing distribution identities and sizes. astral-sh/uv#11239 requests finer-grained
selection for cache removal. astral-sh/uv#9790 tracks the same motivating problem of old versions
accumulating after regular upgrades, while astral-sh/uv#16008 discusses automatically collecting
packages no environment uses.

Current source and documentation establish that `uv cache clean <package>` removes all cache entries
for the named package, not a selected version. `uv cache size` reports only the total size of the
cache. The requested version-level inventory, measurement, and deletion are therefore not current
features. The CLI documentation also discourages symlink mode because cleaning the cache can break
symlinked environments; uv normally shares storage through copy-on-write clones on macOS and Linux
and hardlinks on Windows.

## Draft response

Thanks. The package/version inventory and size reporting are already tracked in
astral-sh/uv#1655, while finer-grained cache removal is tracked in astral-sh/uv#11239. For the
broader problem of old versions accumulating, see astral-sh/uv#9790 and astral-sh/uv#16008.

Today, `uv cache clean <package>` removes all cached entries for that package, and `uv cache size`
reports only the total cache size; version-level listing, sizing, and removal are not currently
available. Also, symlink mode is discouraged because clearing the cache can break environments that
refer to it; the default link modes already share package data where the filesystem supports it.
Let's keep the design discussion in those existing issues.

## Classification

This is a duplicate because the substantive requested capabilities are already tracked by open
issues: astral-sh/uv#1655 covers inspecting cached distributions and their sizes, and
astral-sh/uv#11239 covers selecting cache entries more precisely for removal. The fact that neither
capability is implemented makes the underlying behavior an enhancement, not a correctness bug, but
the duplicate classification takes precedence because those open discussions are suitable places to
centralize the design.

The report's disk-growth trigger is also established in astral-sh/uv#9790, whose reporter describes
weekly upgrades leaving old package versions in an unbounded cache. That issue differs by proposing
automatic retention or eviction. astral-sh/uv#16008 similarly differs by proposing automatic
collection based on whether environments still reference cache contents. A maintainer there notes
that link-count collection does not cover symlinks and favors age- or size-based eviction instead.

## Related

- astral-sh/uv#1655 (open issue), “Add `pip cache info` and `pip cache list`” — The closest overall
  match for inspection: its proposed listing shows cached wheel names, which contain package and
  version identities, together with each entry's size. Its cache-info request also covers aggregate
  cache information. A maintainer confirmed that the information would be useful while noting that
  a new command requires user-experience design.
- astral-sh/uv#11239 (open issue), “Support `uv cache clean` with pattern similar to
  `pip cache remove <pattern>`” — The closest focused match for selective deletion. It requests
  pattern-based removal beyond the existing package-wide `uv cache clean <package>` behavior and is
  labeled as an enhancement and wish.
- astral-sh/uv#9790 (open issue), “uv cache management strategies for always-growing caches?” — It
  reports the same triggering condition: dependency upgrades continually add versions until the
  cache becomes large, while a full clear discards useful artifacts. It asks for automatic latest-only
  retention or LRU eviction rather than the manual package/version commands requested here.
- astral-sh/uv#16008 (open issue), “Should/make `uv cache prune` delete packages that are not used
  anywhere via link count” — It is adjacent to the request to reclaim versions no project uses. Its
  proposed mechanism is automatic link-count-based pruning, and maintainer discussion explains the
  symlink limitation and preference for an age/size-based garbage-collection design.

## Search evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `cache list`, `cache info`, `pip cache list`, `cache clean package`, `cache clean version`,
`pip cache remove`, `specific version`, `package version`, `cache size`, `disk usage`, and cached
package removal. Conceptual queries covered cache inventory and inspection, per-package utilization,
granular cleanup, unused or dangling entries, unbounded cache growth, eviction, garbage collection,
hardlinks, symlinks, and avoiding re-downloads. Fix-oriented searches covered prior and current
implementations of `uv cache clean`, `uv cache prune`, and `uv cache size`, including merged
astral-sh/uv#16032, which added aggregate `uv cache size`; no pull request was found that implements
package/version inventory or version-selective removal.

astral-sh/uv#12854 was inspected because a comment asks for cache usage ordered by package name, but
its issue-level request is aggregate cache utilization and is less precise than astral-sh/uv#1655.
astral-sh/uv#18962 explicitly notes the lack of per-version cleanup, but combines that concern with
temporary-file cleanup and global environment management, so astral-sh/uv#11239 is the more focused
canonical removal request. The historical astral-sh/uv#6909 and merged astral-sh/uv#6915 concern a
bug where package-wide cleaning left dangling archives; that bug was fixed and is not the requested
new version-level selection behavior.
