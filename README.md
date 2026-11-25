# Support 32-bit ARM (armv7) targets in `--python-platform`

Issue: astral-sh/uv#23405

Classification: enhancement

## Summary

The reporter wants x86-64 CI to use `uv sync --frozen --no-dev --no-install-project --no-build` with an explicit 32-bit ARM Linux target, without QEMU. The project routes `psutil` to piwheels for `platform_machine == 'armv7l'`, so the target needs to select that locked branch and accept wheels tagged `linux_armv7l`. The requested spellings include `armv7-unknown-linux-gnueabihf` and `armv7l-manylinux_2_xx`.

The motivating incident was a locked piwheels URL that returned HTTP 404 after files moved. The reporter says frozen sync failed, `uv lock --check` passed, and `uv lock --refresh-package psutil` repaired the lock. The request is for ARMv7 cross-target sync, not for `uv lock --check` to probe remote files.

## Draft response

`--python-platform` doesn't currently accept a 32-bit ARM Linux target, even though uv supports ARMv7 binaries and managed Python downloads. For your CI workflow, the target would need to set `platform_machine` to `armv7l` and accept `linux_armv7l` wheels so that frozen sync selects the piwheels artifacts in your lockfile. In the meantime, run the frozen sync in a fresh ARMv7 environment with an empty cache to check that the wheels can still be downloaded. `uv lock --check` checks the lockfile against the project metadata; it doesn't verify artifact URLs.

## Classification

This is an enhancement: ARMv7 exists in native platform and wheel-tag handling, but `--python-platform` has no ARMv7 target to select. Nothing shows that the flag supported ARMv7 before or that an accepted target now selects the wrong wheels or markers. The URL failure motivates the CI check but does not show that `uv lock --check` behaved incorrectly. The nearest open discussion, astral-sh/uv#18127, notes the missing target but tracks marker values from a running interpreter rather than adding an explicit target.

## Related

- astral-sh/uv#18127 (open issue) — A 32-bit ARM interpreter on an AArch64 kernel reported `aarch64` for `platform.machine()`, affecting piwheels marker selection. A maintainer explicitly distinguished that interpreter-driven path from `--python-platform` and noted the missing 32-bit ARM target. The comments express interest in broader targets, but the issue itself tracks the interpreter-driven behavior.
- astral-sh/uv#21299 (closed issue) — The same gap between internal wheel-tag support and accepted `--python-platform` targets was reported for `s390x` and `ppc64le`, not ARMv7.
- astral-sh/uv#21313 (merged pull request) — Closed astral-sh/uv#21299 by adding GNU Linux targets and manylinux variants with wheel and marker mappings for `s390x`, `ppc64le`, and `loongarch64`. This is an implementation precedent, not an ARMv7 fix.
- astral-sh/uv#14320 (merged pull request) — Added `--python-platform` to project sync for astral-sh/uv#14273. It provides the cross-target sync mechanism this issue wants to use, but did not add an ARMv7 target.
- astral-sh/uv#17384 (merged pull request) — Added `armv8l` as an alias of `armv7l` when parsing wheel platform tags. It fixed native wheel compatibility for astral-sh/uv#17375, not selection of an explicit ARMv7 target.

## Supporting Evidence

- `crates/uv-cli/src/lib.rs:3800` types the sync flag as `TargetTriple`. `crates/uv-configuration/src/target_triple.rs:15` defines the accepted targets and includes ARM64 Linux but no ARMv7 Linux target. The ARMv7 release target and managed Python download records are maintained separately in `docs/reference/policies/platforms.md:57` and `crates/uv-python-managed/download-metadata.json:87`.
- `crates/uv-environment-operations/src/sync.rs:172` derives the marker environment and wheel tags from the target override before selecting from the lock. `crates/uv-configuration/src/target_triple.rs:1872` sets `platform_machine` from the explicit target. `crates/uv-platform-tags/src/platform.rs:293` names the architecture `armv7l`, and `crates/uv-platform-tags/src/tags.rs:592` includes plain `linux_<arch>` as a fallback for manylinux platforms. This supports the requested wheel behavior once an ARMv7 target supplies that architecture; it is not evidence that an ARMv7 override works today.
- `docs/concepts/projects/sync.md:35` defines `uv lock --check` in terms of matching project metadata. It is not a download or artifact reachability check.

## Search Scope

Searched open and closed issues and open, closed, and merged pull requests in astral-sh/uv with authenticated `gh`. Literal searches covered `--python-platform`, both proposed ARMv7 spellings, `linux_armv7l`, piwheels, `platform_machine`, and frozen sync/404/refresh terms. Conceptual searches separated additional Linux targets and cross-install/cross-resolve, marker and wheel-tag compatibility, and stale artifact URLs versus lockfile validation. Checked merged work through commit search, scanned pull request titles across states, and followed candidate comments and issue/PR links. The closest exclusions were astral-sh/uv#6503 (constraining lock resolution, not cross-sync), astral-sh/uv#18166 (cache invalidation for interpreter-derived markers), astral-sh/uv#7957 (broader cross-platform environment construction), and astral-sh/uv#21430 (per-file yanking, not a relocated URL). No earlier item tracks adding the ARMv7 flag values themselves.
