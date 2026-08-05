# Backwards incompatible cache change in uv 0.12.0

Issue: astral-sh/uv#20949
Classification: bug

## Summary

No open duplicate was found. The closest direct change is astral-sh/uv#20443; astral-sh/uv#8367 and astral-sh/uv#8386 establish the governing compatibility guarantee, while astral-sh/uv#12274, astral-sh/uv#12281, astral-sh/uv#19298, and astral-sh/uv#19301 document closely related prior regressions and fixes.

## Classification

This is an established correctness regression, not a duplicate. Repository source confirms that astral-sh/uv#20443 added serialized size fields while leaving the relevant bucket versions unchanged, and the documented cache contract established by astral-sh/uv#8386 guarantees forward and backward compatibility within a bucket version. The reported older-reader failure therefore violates intended behavior. Historical fixes address analogous manifestations, but no open issue or pull request already tracks this uv 0.12.0 recurrence.

## Related

- https://github.com/astral-sh/uv/pull/20443 (merged pull request): Reject pylock artifacts with mismatched declared sizes
  Direct source-backed introducer: astral-sh/uv#20443 added serialized size fields to Archive and Revision while retaining the existing cache buckets. Its legacy tests cover new uv reading old four-field entries, but not older uv reading newly written five-field entries.
- https://github.com/astral-sh/uv/issues/8367 (closed issue): Regression in 0.4.23: data did not match any variant of untagged enum CacheInfoWire
  Canonical earlier discussion of the same trigger: a newer uv writes cache entries that an older uv cannot deserialize. Maintainers concluded that incompatible representations require separate bucket versions.
- https://github.com/astral-sh/uv/pull/8386 (merged pull request): Modify cache versioning to support backwards compatibility
  Historical policy fix for astral-sh/uv#8367. It changed the documented guarantee so representations within one bucket version are forward- and backward-compatible, and bumped the affected bucket.
- https://github.com/astral-sh/uv/issues/12274 (closed issue): uv sync fails due to invalid cache
  Very close prior symptom: mixed uv versions caused Failed to deserialize cache entry followed by array had incorrect length, expected 4 in HttpArchivePointer or LocalArchivePointer handling.
- https://github.com/astral-sh/uv/pull/12281 (merged pull request): Make cache errors non-fatal in Planner::build
  Fix for astral-sh/uv#12274 using both graceful handling of archive-pointer cache failures and a cache-version bump, establishing precedent for this failure class.
- https://github.com/astral-sh/uv/issues/19298 (closed issue): Regression in 0.11.9: `Failed to deserialize cache entry: invalid ID` for IDs written by older uv versions
  Recent analogous regression caused by changing a serialized cache field without bumping sdists-v9. Its direction and exact error differ, but it confirms that an unchanged bucket must retain compatible representations.
- https://github.com/astral-sh/uv/pull/19301 (merged pull request): fix(cache): accept legacy ID format from pre-0.11.9 cache entries (#19298)
  Fix for astral-sh/uv#19298 that restored a representation compatible with the unchanged bucket, providing recent fix-oriented precedent.
