# `uv lock` scales exponentially with the size of a single `[tool.uv] conflicts` group

Issue: astral-sh/uv#21954

Classification: bug

## Summary

The report provides a minimal generator and timing table showing that `uv lock` becomes effectively unusable as the number of extras participating in `[tool.uv] conflicts` grows. With a shared base dependency that has many candidate releases, runtime rises from 0.2 seconds at 20 extras to 109.9 seconds at 65 extras and exceeds 120 seconds at 70. Splitting the same extras across disjoint groups reportedly does not change the behavior, making the total number of distinct conflicted extras the observable trigger.

The closest precedent is astral-sh/uv#16779, fixed by astral-sh/uv#19538, where a large single conflict set caused lock processing to hang after dependency resolution. The new reproduction occurs on uv 0.12.17 and has a different dependency shape, so it is a current bug rather than a duplicate of that closed report. astral-sh/uv#18317 and astral-sh/uv#21399 cover related conflict scaling in workspaces but not this single-project case.

## Draft response

This is a bug, and the reproduction is sufficient to investigate it. astral-sh/uv#16779 previously exposed a combinatorial hang from a large conflict set, and astral-sh/uv#19538 fixed one marker-algebra path. Your reproduction shows that unusable scaling remains on uv 0.12.17 with a different shape: conflicted extras in a single project plus a shared base dependency with many candidate releases. astral-sh/uv#21399 does not cover this case because it removes unrelated extras from conflict simplification while continuing to track extras that participate in conflicts. The next step is to profile this minimal case to identify the remaining hotspot and add a targeted performance regression test.

## Classification

`bug` fits because the supplied minimal reproduction and timing series establish incorrect, effectively unusable lock performance rather than requesting a new capability. The repository already treated the same user-visible failure family as a bug in astral-sh/uv#16779 and astral-sh/uv#19538.

This should not be classified as a duplicate. The historical report is closed, its merged fix memoized `without_extras`, and its regression test used 29 conflicts with one optional dependency that requested another package's extra. The new report reproduces severe growth on uv 0.12.17 using empty conflicted extras and a shared base dependency with a large release history. That difference may exercise another remaining path; the available evidence does not establish that the earlier internal mechanism itself regressed. No open issue was found that tracks this exact single-project trigger.

## Related

- astral-sh/uv#16779 — Closest historical issue. A single project had a group of 26 mutually exclusive extras; resolution completed in seconds, but `uv lock` effectively hung while processing conflict-generated markers. Maintainers traced the work into marker algebra. The issue is closed, so it is historical context rather than an open canonical duplicate.
- astral-sh/uv#19538 — Merged fix for astral-sh/uv#16779, titled “Avoid conflict set combinatorial explosion.” It memoized `without_extras` and added a one-minute timeout regression test using 29 conflicts. The new reproduction demonstrates that a different large-conflict-set shape remains pathological on a later release.
- astral-sh/uv#18317 — Open report about repeated conflict declarations across workspace members. Its comments include lock-time growth and a maintainer explanation that scenario counts grow exponentially, but its primary symptoms are workspace duplication and lockfile bloat rather than runtime from distinct extras in one project.
- astral-sh/uv#21399 — Merged performance improvement for conflict simplification in large workspaces. It stops tracking unrelated extras but explicitly continues tracking extras involved in declared conflicts, so it does not cover the extras in this reproduction.

## Search evidence

Searches covered literal combinations of `conflicts`, `uv lock`, slow or hanging behavior, large conflict sets, many extras, lock time, and exponential growth. Conceptual and fix-oriented searches covered universal-resolution forks, conflict-marker simplification, marker algebra, resolver performance, and merged performance fixes. Both open and closed issues and open, closed, and merged pull requests were searched, and the strongest candidates' comments and linked fixes were inspected.

astral-sh/uv#17990 was inspected but ruled out as a close match because it requests independent resolution targets and environment switching rather than tracking current lock-time scaling. astral-sh/uv#15101 concerns simplifying impossible marker edges and lockfile output, not large-conflict-set runtime. General slow-resolution reports such as astral-sh/uv#10438 and astral-sh/uv#13698 were also ruled out because their confirmed triggers were unrelated resolver forking and index metadata downloads, respectively.
