# packages with android wheels but no sdist fail to resolve

Issue: astral-sh/uv#22805

Classification: bug

## Summary

Both reported configurations reproduce with uv 0.13.4 and CPython 3.14.7 on Linux x86_64. `uv lock` rejects `sherpa-onnx-core==1.13.8` when Android coverage is required, despite the release publishing four Android wheels and no source distribution.

The dependency-marker example also reproduces the reported regression boundary: uv 0.12.13 resolves successfully, while uv 0.12.14 and uv 0.13.4 fail in the Android fork. Removing `tool.uv.environments` from that example lets uv 0.13.4 resolve successfully. Neither reported configuration sets `required-environments`.

## Classification

Bug, with observed reproduction. The package inventory confirms that the rejected Android artifacts exist; this is not simply a request to resolve an unavailable platform. The failure concerns universal locking from a Linux host, not installation or execution on an Android device.

The results are consistent with missing Android platform-marker inference in the resolver and the fork-specific coverage checks introduced by astral-sh/uv#21672. Source inspection supports that explanation, but no implementation change was made or tested to establish a fix. No exact duplicate was identified in the related-item research.

## Reproduction

Outcome: **reproducible**.

### Environment and isolation

- Reported: Linux 7.2.9 x86_64 GNU/Linux, uv 0.13.4, Python 3.14.7.
- Observed: Linux 6.17.0-1022-azure x86_64, uv 0.13.4 (`x86_64-unknown-linux-gnu`), CPython 3.14.7. The different host kernel did not prevent reproduction.
- The main reproduction used the installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.4/x86_64/uv`.
- uv 0.12.13 and uv 0.12.14 binaries were extracted into the temporary directory from their published PyPI Linux x86_64 wheels, after verifying their SHA-256 hashes against PyPI metadata. Both binaries reported the expected versions.
- All project files, per-scenario caches, downloaded Python, comparison binaries, and command logs are under `/tmp/uv-22805-ycn_pcc_`. The runner used an explicit environment with temporary uv/XDG storage, an empty configuration directory and netrc, and disabled keyring lookup. No existing lockfile was supplied to any scenario.
- Public PyPI was selected explicitly. No workspace, dependency groups, custom sources, or build backend is needed.

### Explicit Android environment

Create `pyproject.toml` in a fresh temporary directory:

```toml
[project]
name = "repro"
version = "0"
requires-python = ">=3.14"
dependencies = ["sherpa-onnx-core==1.13.8"]

[tool.uv]
environments = ["sys_platform == 'android'"]
```

With caches and Python storage redirected to temporary directories, run:

```console
uv lock --python 3.14.7 --default-index https://pypi.org/simple
```

uv 0.13.4 exits with status 1:

```text
Using CPython 3.14.7
error: No solution found when resolving dependencies for split (markers: sys_platform == 'android')
  cause: Because sherpa-onnx-core==1.13.8 has no `sys_platform == 'android'`-compatible wheels and your project depends on sherpa-onnx-core==1.13.8, we can conclude that your project's requirements are unsatisfiable.

hint: The resolution failed for an environment that is not the current one, consider limiting the environments with `tool.uv.environments`.
```

Expected: successful locking with the available Android wheels. The observed rejection matches the report. This direct case was tested on uv 0.13.4; its historical introduction was not measured.

### Dependency-created Android fork and version comparison

Create this independent `pyproject.toml` in a fresh temporary directory for each version:

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

Run the same command with each binary:

```console
uv lock --python 3.14.7 --default-index https://pypi.org/simple
/tmp/uv-22805-ycn_pcc_/versions/0.12.13/uv lock --python 3.14.7 --default-index https://pypi.org/simple
/tmp/uv-22805-ycn_pcc_/versions/0.12.14/uv lock --python 3.14.7 --default-index https://pypi.org/simple
```

| uv version | Exit status | Observed result |
| --- | --- | --- |
| 0.12.13 | 0 | `Resolved 4 packages`; lockfile contains both certifi versions with their platform markers and all four Android sherpa wheels. |
| 0.12.14 | 1 | Rejected sherpa in the Android fork with the error below. |
| 0.13.4 | 1 | Same rejection and error as uv 0.12.14. |

```text
Using CPython 3.14.7
error: No solution found when resolving dependencies for split (markers: python_full_version >= '3.14' and sys_platform == 'android')
  cause: Because sherpa-onnx-core==1.13.8 has no `python_full_version >= '3.14'`-compatible wheels and your project depends on sherpa-onnx-core==1.13.8, we can conclude that your project's requirements are unsatisfiable.

hint: The resolution failed for an environment that is not the current one, consider limiting the environments with `tool.uv.environments`.
```

A separate uv 0.13.4 control retained all three dependencies and removed only the `[tool.uv]` environments configuration. The same command exited 0, resolved four packages, and retained all four Android sherpa wheels. This confirms the reported environments trigger for this fixture; it does not validate installation on Android.

### Published artifact inventory

The public PyPI JSON response for `sherpa-onnx-core/1.13.8` lists 13 wheels and zero sdists. Four wheels have these filenames:

```text
sherpa_onnx_core-1.13.8-py3-none-android_24_arm64_v8a.whl
sherpa_onnx_core-1.13.8-py3-none-android_24_armeabi_v7a.whl
sherpa_onnx_core-1.13.8-py3-none-android_24_x86.whl
sherpa_onnx_core-1.13.8-py3-none-android_24_x86_64.whl
```

Both pinned certifi releases are available with universal wheels. The sherpa inventory is retained as `sherpa-onnx-core-1.13.8-inventory.json` in the temporary artifact directory.

### Existing test coverage

Searched the relevant Rust integration tests under `crates/uv/tests/` and `crates/uv-client/tests/it/`, and read the matching setup and assertions.

- `crates/uv/tests/lock/lock.rs`, `lock_android`, gated by `test-universal`: uses `test/scenarios/wheels/android.toml`, sets `environments = ["sys_platform == 'android'"]`, and expects a successful lock retaining Android wheels. Its snapshot also contains a deltachat-rpc-server sdist. It covers Android lockfile filtering with an sdist, not this wheel-only failure.
- `crates/uv/tests/lock/lock.rs`, `lock_required_environment_wheel_url_fork`, also gated by `test-universal`: uses a wheel without an sdist whose tags combine Linux aarch64 and macOS x86_64. With an x86_64 environment and required Linux coverage, it expects a resolution error. It covers fork-specific architecture/platform coverage, not Android inference.

No Android wheel-only regression test was found. Repository test suites were not executed; the evidence above comes from the installed and published uv binaries against isolated copies of the reported projects.

### Retained command evidence

The directories `android-current`, `fork-current`, `fork-0.12.13`, `fork-0.12.14`, and `control-no-environments` each contain the exact `pyproject.toml`, `command.txt`, `stdout.txt`, `stderr.txt`, and `exit-code.txt`; successful scenarios also contain `uv.lock`. The isolation runner is `/tmp/uv-22805-ycn_pcc_/run.py`.

## Related

- astral-sh/uv#21672 — Scope required-environment checks to the current fork (merged pull request). Its resolver diff checks wheel coverage within the current fork. `changelogs/0.12.x.md` places it in uv 0.12.14, released September 15, 2026. The observed version boundary matches this change.
- astral-sh/uv#14977 — Extend wheel filtering to Android tags (merged pull request). Added Android-aware lockfile wheel filtering and `lock_android`, but did not update resolver platform-marker inference. The test includes an sdist. Shipped in uv 0.8.4.
- astral-sh/uv#18451 — Enforce available distributions for supported environments (merged pull request). Extended artifact checks to `tool.uv.environments` as well as `required-environments`, explaining why the reported fixtures exercise coverage checks without the latter. Shipped in uv 0.10.11.
- astral-sh/uv#17109 — uv fails to find wheel for split index/dependency (open issue). Closest analogous failure: existing FreeBSD wheels are rejected under required environments because their tags lack marker mappings. It concerns a different platform and split indexes, not this Android regression.

## Implementation evidence and next step

The checked-out implementation at commit `75698b1` is consistent with the observed failures:

- `crates/uv-resolver/src/prioritized_distribution.rs`, `implied_platform_markers`: starts with false coverage and handles universal, Windows, macOS, and Linux tags. Android reaches the fallback logging “Unknown platform tag in wheel tag” without adding coverage.
- The same file's `insert_source` marks Python and platform coverage as universally satisfied when a compatible sdist exists. That explains the coverage gap in `lock_android`.
- `crates/uv-lock-operations/src/lock.rs` combines supported and required environments into `artifact_environments`.
- `crates/uv-resolver/src/resolver/mod.rs`, `fork_version_registry`: checks artifact coverage within the current fork. The inspected diff for astral-sh/uv#21672 introduces this fork-sensitive check.
- `crates/uv-lock/src/lock/mod.rs` separately supports Android wheel filtering. Successful control lockfiles retain the Android artifacts.

Investigate Android wheel-to-marker inference and add separate named integration tests for explicit Android coverage and a dependency-created Android fork, using wheel-only fixtures in the existing lock module and the neighboring `test-universal` gate. Check architecture mappings against Android interpreter marker values. No patch or native Android installation was tested.

## Related-item search context

Prior related-item research searched open and closed issues and pull requests for Android wheels without sdists, sherpa-onnx-core, platform-marker inference, required environments, fork checks, and the uv 0.12.14 boundary. It found no exact duplicate. The relevant implementation, release notes, tests, and astral-sh/uv#21672 resolver diff were inspected before selecting the reproduction fixtures.

Other references retained from that research:

- astral-sh/uv#20602 migrated `lock_android` to Packse to avoid mutable PyPI state; it did not add the wheel-only case.
- astral-sh/uv#17382 and astral-sh/uv#20383 discuss/document limited platform mappings. Superseded astral-sh/uv#17118 led to merged astral-sh/uv#18451.
- astral-sh/uv#8029 and astral-sh/uv#15646 concern Android installation tags. astral-sh/uv#18301 and astral-sh/uv#18302 concern native Android interpreter recognition and integration tests.
- Excluded as different failures: astral-sh/uv#9711 (incomplete resolutions), astral-sh/uv#17178 (Termux markers), astral-sh/uv#19911 (Requires-Python metadata), astral-sh/uv#22716 (absent Windows wheels), astral-sh/uv#14040 and astral-sh/uv#14041 (Windows AMD64 mapping), astral-sh/uv#21753 (incompatible source artifacts), and astral-sh/uv#21825 (macOS release boundaries).

No checkout files or GitHub state were changed. Only this issue-context README and temporary reproduction artifacts were written.
