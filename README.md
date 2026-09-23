# `uv lock` scales exponentially with the size of a single `[tool.uv] conflicts` group

Issue: astral-sh/uv#21954

Classification: bug

## Summary

The reported scaling is reproducible. In an isolated temporary project with `transformers>=4.40.0,<6.0.0` as a base dependency and empty extras in one conflict set, wall time for `uv lock` grew from 24 ms at 5 extras to 28.901 seconds at 50 extras. A 50-extra control with no base dependency completed in 50 ms. The lock command's own resolver timing remained below 150 ms at every measured size, while later resolution-output and lock construction processed the redundant forks.

The root cause was the workspace-member requirement producer: before universal lock resolution, `ExtrasResolver` expanded every name in package metadata's `Provides-Extra` into a requested extra, including extras with no conditional requirements. Conflict handling therefore created a full resolver environment for each empty extra. Every environment contained the same shared base dependency graph, and downstream conflict-marker processing repeatedly combined those equivalent paths.

The issue reports uv 0.12.17 on macOS arm64; the independent reproduction used the installed uv 0.12.13 on Linux x86_64 and CPython 3.12.3. The closest precedent is astral-sh/uv#16779, fixed by astral-sh/uv#19538, but that regression fixture has a different dependency shape. astral-sh/uv#18317 and astral-sh/uv#21399 cover related conflict scaling in workspaces but not this single-project case.

## Reproduction

Outcome: `reproducible`.

All files, the uv cache, and the Python installation directory were placed under `/tmp/uv-issue-21954.wVYxMw`. The runner's unrelated `UV_LOCKED=1` setting was overridden with `UV_LOCKED=false`. After warming the isolated cache, each independently generated project was timed with:

```console
UV_LOCKED=false \
UV_CACHE_DIR=/tmp/uv-issue-21954.wVYxMw/cache \
UV_PYTHON_INSTALL_DIR=/tmp/uv-issue-21954.wVYxMw/python \
UV_NO_PROGRESS=1 \
timeout 150 uv lock --project /tmp/uv-issue-21954.wVYxMw/n<N>
```

Each project used this shape, with `e0` through `e<N-1>` declared as empty optional dependencies and all included in the one conflict set:

```toml
[project]
name = "repro"
version = "0.0.1"
requires-python = ">=3.10,<3.15"
dependencies = ["transformers>=4.40.0,<6.0.0"]

[project.optional-dependencies]
e0 = []
e1 = []
# ...

[tool.uv]
conflicts = [[
  { extra = "e0" },
  { extra = "e1" },
  # ...
]]
```

Observed wall times were:

| Extras | Wall time | uv-reported resolver time |
| ---: | ---: | ---: |
| 5 | 24 ms | 12 ms |
| 10 | 38 ms | 22 ms |
| 20 | 194 ms | 46 ms |
| 30 | 1.523 s | 81 ms |
| 40 | 7.476 s | 110 ms |
| 50 | 28.901 s | 147 ms |

All six locks exited successfully. Replacing the `transformers` requirement with `dependencies = []` in the 50-extra project reduced wall time to 50 ms and resolved one package in 19 ms. This confirms the reported severe nonlinear scaling for the supplied project shape; the separate claim that splitting the extras across several conflict sets performs identically was not tested.

Existing integration coverage at `crates/uv/tests/lock_scenarios/lock_conflict.rs`, `many_conflicts_with_requested_dependency_extra`, verifies that a different 29-conflict fixture completes within one minute. It uses one non-empty optional dependency whose local transitive dependency requests an extra. It does not cover empty conflicted extras combined with a shared base dependency having a large release history, nor does it assert scaling across conflict counts.

## Fix

Outcome: fixed.

`crates/uv-requirements/src/extras.rs` now activates only `Provides-Extra` entries that contribute an optional requirement after marker simplification. Empty extras remain in distribution metadata and the lockfile's `provides-extras`, and declared conflicts remain available for install-time validation, but empty extras no longer create resolver forks.

The parent regression in `crates/uv/tests/lock/lock.rs`, `lock_conflicting_empty_extras_fork_shared_dependency`, now requires one search for the shared dependency and no conflict split or multi-environment completion message. Before the production change, that desired snapshot failed because four empty extras produced five environments and five identical searches. Existing producer/consumer coverage confirms that removing an empty extra still invalidates a lock, a metadata-free frozen lock can still select an empty extra, and non-empty mixed extra/group conflicts still resolve, lock, and sync correctly.

Successful focused validation:

- `cargo test -p uv --test lock lock::lock_conflicting` — all 10 conflict-lock integration tests passed.
- `cargo test -p uv --test lock lock::lock_metadata_free_frozen_empty_extra -- --exact` — passed.
- `cargo test -p uv --test lock lock::lock_removed_empty_extra -- --exact` — passed.
- `rustup run stable cargo clippy -p uv-requirements --all-targets --locked -- -D warnings` — passed.
- `rustup run stable cargo fmt --all` — completed successfully.

## Draft response

This was reproducible and is fixed by excluding dependency-free extras from the set activated for universal lock resolution. Declared empty extras and their conflicts remain represented in project metadata and lock validation, but they no longer create equivalent resolver environments around shared dependencies. The regression fixture now performs one shared-dependency search instead of five searches across five environments.

## Classification

`bug` fits because a targeted reproduction confirms effectively unusable lock performance rather than a request for a new capability. The repository already treated the same user-visible failure family as a bug in astral-sh/uv#16779 and astral-sh/uv#19538.

This should not be classified as a duplicate. The historical report is closed, its merged fix memoized `without_extras`, and its regression test used 29 conflicts with one optional dependency that requested another package's extra. This issue was caused earlier in the lock pipeline by activating dependency-free `Provides-Extra` entries as root requirements. No open issue was found that tracks this exact single-project trigger.

## Related

- astral-sh/uv#16779 — Closest historical issue. A single project had a group of 26 mutually exclusive extras; resolution completed in seconds, but `uv lock` effectively hung while processing conflict-generated markers. Maintainers traced the work into marker algebra. The issue is closed, so it is historical context rather than an open canonical duplicate.
- astral-sh/uv#19538 — Merged fix for astral-sh/uv#16779, titled “Avoid conflict set combinatorial explosion.” It memoized `without_extras` and added a one-minute timeout regression test using 29 conflicts. The new reproduction demonstrates that a different large-conflict-set shape remains pathological in uv 0.12.13 and is reported in uv 0.12.17.
- astral-sh/uv#18317 — Open report about repeated conflict declarations across workspace members. Its comments include lock-time growth and a maintainer explanation that scenario counts grow exponentially, but its primary symptoms are workspace duplication and lockfile bloat rather than runtime from distinct extras in one project.
- astral-sh/uv#21399 — Merged performance improvement for conflict simplification in large workspaces. It stops tracking unrelated extras but explicitly continues tracking extras involved in declared conflicts, so it does not cover the extras in this reproduction.

## Search evidence

Searches covered literal combinations of `conflicts`, `uv lock`, slow or hanging behavior, large conflict sets, many extras, lock time, and exponential growth. Conceptual and fix-oriented searches covered universal-resolution forks, conflict-marker simplification, marker algebra, resolver performance, and merged performance fixes. Both open and closed issues and open, closed, and merged pull requests were searched, and the strongest candidates' comments and linked fixes were inspected.

astral-sh/uv#17990 was inspected but ruled out as a close match because it requests independent resolution targets and environment switching rather than tracking current lock-time scaling. astral-sh/uv#15101 concerns simplifying impossible marker edges and lockfile output, not large-conflict-set runtime. General slow-resolution reports such as astral-sh/uv#10438 and astral-sh/uv#13698 were also ruled out because their confirmed triggers were unrelated resolver forking and index metadata downloads, respectively.

Pull request: https://github.com/astral-sh/uv-dev/pull/2065
