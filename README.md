# packages with android wheels but no sdist fail to resolve

Issue: astral-sh/uv#22805

Classification: bug

## Summary

Both reported configurations reproduce with uv 0.13.4 and CPython 3.14.7 on Linux x86_64. `uv lock` rejects `sherpa-onnx-core==1.13.8` when Android coverage is required, despite the release publishing four Android wheels and no source distribution.

The dependency-marker example also reproduces the reported regression boundary: uv 0.12.13 resolves successfully, while uv 0.12.14 and uv 0.13.4 fail in the Android fork. Removing `tool.uv.environments` from that example lets uv 0.13.4 resolve successfully. Neither reported configuration sets `required-environments`. The working tree now fixes Android wheel coverage and retains the parent regressions as success tests; see the Fix section.

## Classification

Bug, with observed reproduction. The package inventory confirms that the rejected Android artifacts exist; this is not simply a request to resolve an unavailable platform. The failure concerns universal locking from a Linux host, not installation or execution on an Android device.

The missing Android platform-marker inference is confirmed by the parent regressions and the focused fix: all three tests pass while asserting rejection, fail when changed to expect success before the production change, and pass with Android marker inference implemented. The fork-specific regression boundary matches astral-sh/uv#21672. No exact duplicate was identified in the related-item research.

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

The initial investigation found no Android wheel-only regression test. The parent regression pull request, astral-sh/uv-dev#2927, subsequently added three tests in `crates/uv/tests/lock/lock.rs`: `lock_android_wheel_only`, `lock_android_wheel_only_fork`, and `lock_android_wheel_url_required_environment`. They initially asserted the incorrect rejection. All three are retained and updated by the fix described below. The published-version reproduction above remains distinct from the debug-profile tests used to validate the fix.

### Retained command evidence

The directories `android-current`, `fork-current`, `fork-0.12.13`, `fork-0.12.14`, and `control-no-environments` each contain the exact `pyproject.toml`, `command.txt`, `stdout.txt`, `stderr.txt`, and `exit-code.txt`; successful scenarios also contain `uv.lock`. The isolation runner is `/tmp/uv-22805-ycn_pcc_/run.py`.

## Related

- astral-sh/uv-dev#2927 — Parent regression pull request. Added the three wheel-only integration tests, initially passing while asserting the reported resolver errors.

- astral-sh/uv#21672 — Scope required-environment checks to the current fork (merged pull request). Its resolver diff checks wheel coverage within the current fork. `changelogs/0.12.x.md` places it in uv 0.12.14, released September 15, 2026. The observed version boundary matches this change.
- astral-sh/uv#14977 — Extend wheel filtering to Android tags (merged pull request). Added Android-aware lockfile wheel filtering and `lock_android`, but did not update resolver platform-marker inference. The test includes an sdist. Shipped in uv 0.8.4.
- astral-sh/uv#18451 — Enforce available distributions for supported environments (merged pull request). Extended artifact checks to `tool.uv.environments` as well as `required-environments`, explaining why the reported fixtures exercise coverage checks without the latter. Shipped in uv 0.10.11.
- astral-sh/uv#17109 — uv fails to find wheel for split index/dependency (open issue). Closest analogous failure: existing FreeBSD wheels are rejected under required environments because their tags lack marker mappings. It concerns a different platform and split indexes, not this Android regression.

## Implementation evidence

The implementation before the fix, at commit `75698b1`, explains the observed failures:

- `crates/uv-resolver/src/prioritized_distribution.rs`, `implied_platform_markers`: starts with false coverage and handles universal, Windows, macOS, and Linux tags. Android reaches the fallback logging “Unknown platform tag in wheel tag” without adding coverage.
- The same file's `insert_source` marks Python and platform coverage as universally satisfied when a compatible sdist exists. That explains the coverage gap in `lock_android`.
- `crates/uv-lock-operations/src/lock.rs` combines supported and required environments into `artifact_environments`.
- `crates/uv-resolver/src/resolver/mod.rs`, `fork_version_registry`: checks artifact coverage within the current fork. The inspected diff for astral-sh/uv#21672 introduces this fork-sensitive check.
- `crates/uv-lock/src/lock/mod.rs` separately supports Android wheel filtering. Successful control lockfiles retain the Android artifacts.

The focused fix below implements Android marker inference and updates the parent wheel-only tests. Native Android installation was not tested; the affected feature is universal locking from a Linux host.

## Related-item search context

Prior related-item research searched open and closed issues and pull requests for Android wheels without sdists, sherpa-onnx-core, platform-marker inference, required environments, fork checks, and the uv 0.12.14 boundary. It found no exact duplicate. The relevant implementation, release notes, tests, and astral-sh/uv#21672 resolver diff were inspected before selecting the reproduction fixtures.

Other references retained from that research:

- astral-sh/uv#20602 migrated `lock_android` to Packse to avoid mutable PyPI state; it did not add the wheel-only case.
- astral-sh/uv#17382 and astral-sh/uv#20383 discuss/document limited platform mappings. Superseded astral-sh/uv#17118 led to merged astral-sh/uv#18451.
- astral-sh/uv#8029 and astral-sh/uv#15646 concern Android installation tags. astral-sh/uv#18301 and astral-sh/uv#18302 concern native Android interpreter recognition and integration tests.
- Excluded as different failures: astral-sh/uv#9711 (incomplete resolutions), astral-sh/uv#17178 (Termux markers), astral-sh/uv#19911 (Requires-Python metadata), astral-sh/uv#22716 (absent Windows wheels), astral-sh/uv#14040 and astral-sh/uv#14041 (Windows AMD64 mapping), astral-sh/uv#21753 (incompatible source artifacts), and astral-sh/uv#21825 (macOS release boundaries).

The initial reproduction changed no checkout files or GitHub state. The fix changes only the two production files and the parent integration-test file listed below. No commits, pushes, Git configuration changes, or GitHub mutations were made.


## Fix

Outcome: **fixed** in the working tree.

### Implementation

- `crates/uv-resolver/src/prioritized_distribution.rs`: infer `sys_platform == 'android'` and the appropriate `platform_machine` constraint from Android wheel tags. Both registry artifact coverage and direct-wheel URL checks consume this inference; no separate resolver or lockfile-filter rewrite is needed.
- `crates/uv-platform-tags/src/tags.rs`: expose the Android ABI-to-machine mapping (`arm64_v8a` → `aarch64`, `armeabi_v7a` → `armv7l`, `x86` → `i686`, `x86_64` → `x86_64`) with an exhaustive enum match. These names agree with the interpreter's existing Android architecture normalization.
- `crates/uv/tests/lock/lock.rs`: retain all three parent regressions and change their error snapshots to successful resolution. Add full lockfile snapshots retaining the Android wheels and the platform-specific dependency pins. The explicit Android environment and direct-URL cases validate their generated locks with `--locked --offline --no-cache`; the dependency-fork case consumes its lock with frozen, offline, cache-free export and checks both platform-specific pins. All existing feature gates remain intact. The original Android-with-sdist test remains unchanged.

### Scope investigation

Inspected supported versus required environments, registry and direct-wheel coverage paths, platform/ABI inference, Android lockfile filtering, and nearby tests in the lock, pip-compile, sync, and project-export integration modules. The three parent cases already represent the demonstrated Android failures, so no additional named tests or unrelated fixture changes were needed. Existing non-Android missing-wheel expectations do not contradict this fix.

The Python-only environment in the fork fixture revealed a separate lock-validation limitation: it serializes to an empty supported-marker list but is compared with `[true]` on re-read, triggering re-resolution. A dependency-free project with the same Python restriction reproduced that mismatch using the installed uv 0.13.4, without Android wheels or the production patch. Debug output explicitly reports `change in supported environments: [] vs. [true]`. This has a different cause and was left outside this fix. Frozen export validates consumption of the forked lock without invoking that normalization path.

### Focused validation

On Linux x86_64 with CPython 3.14.8 and Rust 1.99.0:

- Before production changes, `cargo test --locked -p uv --test lock lock_android_wheel -- --nocapture` passed all three parent tests while they asserted the undesirable errors.
- After changing only those expectations to success, the same focused selection failed all three tests with the reported Android-coverage errors, confirming the regressions detect the bug.
- `INSTA_UPDATE=no cargo test --locked -p uv --test lock lock_android` passed all four Android tests after the fix, including the original sdist case and the lockfile producer/consumer checks.
- `INSTA_UPDATE=no cargo test --locked -p uv --test lock -- lock::lock_required_environment_python_fork lock::lock_required_environment_wheel_url_fork --exact` passed both neighboring coverage tests.
- `RUSTUP_TOOLCHAIN=stable cargo clippy --locked -p uv -p uv-resolver -p uv-platform-tags --lib -- -D warnings` passed with the application feature set.
- `RUSTUP_TOOLCHAIN=stable cargo fmt --all`, its `-- --check` verification, and `git diff --check` passed. The installed stable toolchain is also Rust 1.99.0; it supplies rustfmt absent from the pinned toolchain installation.

All end-to-end tests used the unoptimized debug test profile and local Packse fixtures. No full test suite or release build was run.

Pull request: https://github.com/astral-sh/uv-dev/pull/2928
