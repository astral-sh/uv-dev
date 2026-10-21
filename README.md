# packages with android wheels but no sdist fail to resolve

Issue: astral-sh/uv#22805

Classification: bug

## Summary

`uv lock` rejects Android wheels for a package without a source distribution when the resolution requires Android coverage. The reporter uses `sherpa-onnx-core==1.13.8`, Python 3.14.7, and uv 0.13.4 on Linux x86_64. They report Android wheels for arm64_v8a, armeabi_v7a, x86, and x86_64.

There are two distinct cases: an explicit Android supported environment fails directly, and a dependency marker creates an Android fork that reportedly succeeds with uv 0.12.13 but fails with uv 0.12.14 and later. Source inspection confirms that Android tags are absent from resolver platform-marker inference, despite support in lockfile wheel filtering. No exact duplicate was found.

astral-sh/uv#21672 explains the reported regression boundary, astral-sh/uv#14977 establishes the incomplete Android coverage, and astral-sh/uv#18451 explains the environments trigger. astral-sh/uv#17109 is a FreeBSD analogue. No exact duplicate found.

## Reported reproductions

For the first case, run `uv lock` with:

```toml
[project]
name = "repro"
version = "0"
requires-python = ">=3.14"
dependencies = ["sherpa-onnx-core==1.13.8"]

[tool.uv]
environments = ["sys_platform == 'android'"]
```

The reported error says the package has no `sys_platform == 'android'`-compatible wheels. The expected result is a successful lock retaining compatible Android wheels.

For the fork-specific case, run `uv lock` with:

```toml
[project]
name = "repro"
version = "0"
requires-python = ">=3.14"
dependencies = [
    "sherpa-onnx-core==1.13.8",
    "certifi==2026.5.20; sys_platform != 'android'",
    "certifi==2026.7.22; sys_platform == 'android'",
]

[tool.uv]
environments = ["python_version >= '3.14'"]
```

The reported failure occurs in the split `python_full_version >= '3.14' and sys_platform == 'android'`, while the cause says the package has no `python_full_version >= '3.14'`-compatible wheels. Neither reproduction explicitly sets `required-environments`.

## Draft response

Your diagnosis matches the resolver code: Android wheel tags are missing from implied_platform_markers, while a compatible sdist supplies coverage for every environment and masks the omission in lock_android. astral-sh/uv#14977 added Android lockfile filtering; astral-sh/uv#21672 made coverage checks apply within each fork in uv 0.12.14, consistent with your reported regression. These checks also apply to tool.uv.environments. The next step is to add the Android wheel-to-marker mapping and regression tests without an sdist for both the explicit Android environment and the dependency-fork example.

## Classification

The source confirms that implied_platform_markers omits Android, so Android wheels contribute no platform coverage; a compatible sdist masks the error. This incorrectly rejects available wheels. The reported regression from uv 0.12.13 to 0.12.14 aligns with astral-sh/uv#21672 and its release notes, although the reproductions were not executed. No existing item found tracks this Android failure closely enough to centralize it as a duplicate.

The missing mapping is a correctness problem even though other discussions acknowledge limitations in platform inference. The FreeBSD report concerns a different platform mapping, and the Android installation-tag work did not fix this resolver path. There is no evidence that a previous Android marker-inference fix was subsequently reverted; the reported regression exposes an existing omission through stricter fork checks.

## Related

- astral-sh/uv#21672 — Scope required-environment checks to the current fork (merged pull request). Changed wheel-coverage checks to intersect the current resolver fork. Shipped in uv 0.12.14, matching the reported regression boundary: non-Android wheels can no longer satisfy coverage in the Android fork. This change exposes the missing Android mapping; it did not implement that mapping.
- astral-sh/uv#14977 — Extend wheel filtering to Android tags (merged pull request). Added Android-aware lockfile wheel filtering and lock_android, but did not update the resolver's implied_platform_markers. The test includes an sdist, which supplies universal coverage and masks the reported omission.
- astral-sh/uv#18451 — Enforce available distributions for supported environments (merged pull request). Extended artifact-availability checks to tool.uv.environments as well as required-environments. This explains why both reproductions exercise coverage checks without explicitly setting required-environments; the current lock configuration combines both lists.
- astral-sh/uv#17109 — uv fails to find wheel for split index/dependency (open issue). Closest analogous failure: existing FreeBSD wheels are rejected under required-environments because their tags lack marker mappings. A maintainer confirms that these mappings are manual. It concerns FreeBSD and split indexes, not the Android mapping or the uv 0.12.14 regression.

## Supporting evidence

- `crates/uv-resolver/src/prioritized_distribution.rs:1016`: `implied_platform_markers` starts with `MarkerTree::FALSE` and handles universal, Windows, macOS, and Linux tags. Android reaches the fallback that logs “Unknown platform tag in wheel tag” without adding coverage. A wheel carrying only Android platform tags therefore contributes false platform coverage.
- `crates/uv-resolver/src/prioritized_distribution.rs:535`: inserting a compatible sdist sets Python and platform coverage to `MarkerTree::TRUE`. This explains why the no-sdist condition matters.
- `crates/uv-lock-operations/src/lock.rs:638`: supported environments and required environments are combined into `artifact_environments`, which is passed to the resolver. The history in astral-sh/uv#18451 confirms that this is intentional and shipped in uv 0.10.11.
- `crates/uv-resolver/src/resolver/mod.rs:1627`: registry candidates are checked for artifact coverage within the current fork. The direct-wheel path at line 1271 uses the same fork-sensitive principle. The diff for astral-sh/uv#21672 confirms this change; `changelogs/0.12.x.md:969` places it in uv 0.12.14, released September 15, 2026.
- `crates/uv-lock/src/lock/mod.rs:152` defines Android markers, and line 10497 applies Android-specific filtering. The diff for astral-sh/uv#14977 added this filtering and the Android lock test, without changing resolver marker inference. It shipped in uv 0.8.4.
- `crates/uv/tests/lock/lock.rs:43339`: `lock_android` uses `test/scenarios/wheels/android.toml`; its snapshot includes an sdist as well as Android wheels. astral-sh/uv#20602 migrated this test to Packse to avoid mutable PyPI state, without covering the wheel-only case.
- Existing fork tests at `crates/uv/tests/lock/lock.rs:44014` and `crates/uv/tests/lock/lock.rs:44442` cover Python and architecture forks with Linux/Windows/macOS wheels. They do not cover the missing Android mapping.

## Search coverage and excluded candidates

Used authenticated gh to search open and closed issues and open, closed, and merged PRs. Separated Android wheel-only rejection from the fork-specific version regression; searched sherpa-onnx-core, Android/sdist, sys_platform, compatible-wheels errors, Unknown platform tag in wheel tag, implied_platform_markers, required-environments/forks, and 0.12.14. Broadened to wheel coverage, platform-marker inference, source-distribution absence, and historical fixes. PR search omitted known matches, so supplemented it with paginated PR title/body searches and direct inspection of linked PRs, comments, timelines, diffs, and release notes. Ruled out astral-sh/uv#9711 (accepting incomplete resolutions), astral-sh/uv#17178 (Termux marker availability), astral-sh/uv#19911 (Requires-Python metadata), astral-sh/uv#22716 (genuinely absent Windows dependency wheels), and astral-sh/uv#14040 with astral-sh/uv#14041 (fixed Windows AMD64 mapping). Android support in astral-sh/uv#8029 and astral-sh/uv#15646 concerns installation tags, not this resolver omission.

The comment chain from astral-sh/uv#17109 leads to astral-sh/uv#17382 and the open documentation PR astral-sh/uv#20383. Those describe the limited platform mappings; they do not implement Android coverage. The superseded astral-sh/uv#17118 led to merged astral-sh/uv#18451, establishing why supported environments now require artifacts.

Also inspected astral-sh/uv#18301 and astral-sh/uv#18302: those concern native Android interpreter recognition and integration testing. They do not address universal locking from a Linux host. Nearby resolver fixes astral-sh/uv#21753 and astral-sh/uv#21825 concern incompatible source artifacts and macOS release boundaries, respectively.

## Recommended next step and validation limits

Add Android wheel-to-marker inference, then cover the explicit Android environment and the dependency-created Android fork with separate named integration tests using packages without sdists. Use the existing lock test module, Packse fixtures, explicit invocations, and snapshots, with the neighboring `test-universal` feature gate. Check architecture mapping against Android interpreter marker values.

The source, existing tests, relevant PR diffs, comments, and release notes were inspected. The reported commands were not executed, and the package file inventory and cross-version result were not independently verified. No checkout files or GitHub state were changed.
