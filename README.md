# `--python-platform x86_64-pc-windows-msvc` sets `platform_machine` to `x86_64` instead of `AMD64`

Issue: astral-sh/uv#22920

Classification: **bug; fixed in the working tree**

## Summary

Reproduced with the installed uv 0.13.5 on Linux x86_64. Both `--python-platform x86_64-pc-windows-msvc` and `--python-platform windows` reject an empty `pylock.toml` restricted to `sys_platform == 'win32' and platform_machine == 'AMD64'`. Changing only `AMD64` to `x86_64` makes both commands succeed. This conflicts with uv's documented Windows x64 marker value, `AMD64`. The working-tree fix changes the shared Windows x64 target mapping to `AMD64` and updates all three parent regression tests to assert correct environment validation, conditional package selection, and compile-to-pylock-to-sync behavior.

The reporter used Python 3.15.1; the reproduction used CPython 3.12.3 while retaining the reported `--python-version 3.11` target override. The mismatch therefore also occurs with a different local interpreter. The reporter additionally describes skipped conditional packages, nab-generated locks, and a pipx zipapp workaround; conditional package selection is now verified by the parent integration tests, while the nab and pipx workflows were not independently exercised.

## Classification

**Bug, reproduced and fixed in the working tree.** The observed environment validation distinguishes the two marker spellings in the reported direction, and inspection of the original source confirms that both Windows x64 target names mapped to `x86_64`. The documented Windows x64 value and the existing Windows wheel-implied marker are `AMD64`.

No regression from a known-good target mapping or exact duplicate has been established. The report does not identify a last known-good version. The pylock validation introduced by astral-sh/uv#22594 and listed in the uv 0.13.2 release notes exposes this inconsistent target marker; it did not change the target mapping. The historical fix in astral-sh/uv#14041 corrected markers inferred from wheels, a separate path whose correction remains present.

The 32-bit discussion in astral-sh/uv#16576 concerns which host architecture to assume for an i686 target. This report concerns the spelling of the explicit x64 target value. Likewise, astral-sh/uv#18298 concerns a user-written `x86_64` requirement, whereas here uv supplies that value itself.

## Reproduction

Outcome: **reproducible** with uv 0.13.5 before the fix. The initial CLI reproduction is recorded below; subsequent debug-build regression validation is described in **Fix**.

### Environment and isolation

- Installed executable on PATH: `/opt/hostedtoolcache/uv/0.13.5/x86_64/uv`, reporting `uv 0.13.5 (x86_64-unknown-linux-gnu)`.
- Host: Ubuntu 24.04.5 LTS, Linux `6.17.0-1022-azure`, x86_64. The report used Linux `7.2.11-300.fc45.x86_64`.
- Selected interpreter: CPython 3.12.3 at `/usr/bin/python`; target Python override: `3.11`.
- Fixtures and caches: `/tmp/uv-22920-64dx7cyw`. Commands ran from separate `amd64` and `x86_64` fixture directories with a clean environment (`env -i`), explicit PATH, `UV_PYTHON_DOWNLOADS=never`, and `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, and `TMPDIR` directed beneath that temporary directory. `--no-config` disabled configuration discovery.
- The empty lockfile requires no package downloads or builds. The initial CLI reproduction changed no repository files or existing user environments. The subsequent fix changes only the production mapping and the two parent integration-test files; no GitHub changes were made.

### Minimal fixture and commands

Create `pylock.toml` in a temporary working directory:

```toml
lock-version = "1.0"
environments = ["sys_platform == 'win32' and platform_machine == 'AMD64'"]
requires-python = ">=3.11"
created-by = "test"
packages = []
```

Run with a temporary uv cache and target directory:

```sh
uv pip install --no-config --preview-features pylock --dry-run --target t --python-version 3.11 --python-platform x86_64-pc-windows-msvc -r pylock.toml
uv pip install --no-config --preview-features pylock --dry-run --target t --python-version 3.11 --python-platform windows -r pylock.toml
```

Both commands exit with status 2 and print:

```text
Using CPython 3.12.3 interpreter at: /usr/bin/python
error: The target environment is not supported by the lockfile
```

In a second temporary fixture, change only `AMD64` to `x86_64` in the environment expression and run the same two commands. Both exit with status 0 and print:

```text
Using CPython 3.12.3 interpreter at: /usr/bin/python
Checked in 0.00ms
Would make no changes
```

| Target | Lockfile `platform_machine` | Exit status | Observed result |
| --- | --- | --- | --- |
| `x86_64-pc-windows-msvc` | `AMD64` | 2 | Target environment rejected |
| `x86_64-pc-windows-msvc` | `x86_64` | 0 | Accepted; would make no changes |
| `windows` | `AMD64` | 2 | Target environment rejected |
| `windows` | `x86_64` | 0 | Accepted; would make no changes |

Expected behavior is acceptance of the Windows x64 `AMD64` environment for both target names. The four command results establish the reported mismatch without relying on source inspection alone. Native Windows interpreter behavior, Python 3.15.1, older uv versions, and downstream workflows were not tested.

### Existing test coverage

Searched relevant tests under `crates/uv/tests/` and `crates/uv-client/tests/it/` and read the following fixtures and assertions:

- `crates/uv/tests/pip_install/pip_install.rs`, `pep_751_incompatible_environment`: uses Python 3.12 with an empty pylock restricted to `python_version < '3.12'` and asserts exit 2 with the same environment error. It covers rejection of a genuinely incompatible Python environment, without a Windows target override.
- `crates/uv/tests/lock/lock.rs`, `windows_amd64_required` (`test-universal`): locks `pywin32` with Windows `x86` and `AMD64` required environments, asserts successful resolution, and snapshots the required markers and wheels. It covers wheel compatibility during universal locking, not explicit target marker evaluation during installation.
- `crates/uv/tests/pip_compile/pip_compile.rs`, `python_platform`: compiles `black` for Linux ARM64 and Windows x64, asserting six and seven resolved packages respectively, including `colorama` on Windows. Its fixture and snapshots do not test an `AMD64` marker restriction.

The initial search found no focused test of this override. The parent regression pull request, astral-sh/uv-dev#2995, subsequently added three tests, all retained and updated by this fix:

- `crates/uv/tests/pip_install/pip_install.rs`, `pep_751_windows_amd64_environment`: accepts `AMD64` and rejects `x86_64` for the explicit Windows x64 target.
- `crates/uv/tests/pip_install/pip_install.rs`, `pep_751_windows_amd64_package_marker`: selects an `AMD64`-conditional package from a pylock instead of silently skipping it.
- `crates/uv/tests/pip_compile/pip_compile.rs`, `pep_751_windows_amd64_compile`: retains an `AMD64`-conditional requirement in a generated pylock and checks that syncing that file selects the package.

These tests are gated by their modules' existing `test-python` and `test-pypi` feature requirements. No new tests or additional feature gates were needed.

## Related

- astral-sh/uv-dev#2995 (parent regression pull request) — Adds the three passing tests that asserted the undesirable behavior. Both original test files are updated by the fix.

- astral-sh/uv#16576 (issue, open) — Unexpected dependency resolving with `platform_machine` and `--python-platform`. Closest command-level discussion: a 32-bit Windows target supplies x86 while a 32-bit interpreter on a 64-bit host reports AMD64, changing dependency selection. Maintainers discuss the ambiguous host architecture and require overrides to be host-independent. This does not establish the separate x64 target spelling bug as a duplicate.
- astral-sh/uv#22594 (pull request, merged) — Honor `pylock.toml` environment restrictions during installation. Merged October 15, 2026, and released in uv 0.13.2, adding the exact reported error and evaluation of pylock environments against the target markers. This exposes the incorrect Windows x64 mapping for pylock installs; it neither changes nor fixes that mapping.
- astral-sh/uv#18298 (issue, open) — false positive error installing playwright when `required-environments = ["sys_platform == 'win32' and platform_machine == 'x86_64'"]`. Maintainers confirm AMD64 is the Windows x64 marker expected by wheel compatibility checks. This concerns a user-supplied x86_64 marker in required-environments, whereas the new report concerns uv itself generating that incorrect value through --python-platform.
- astral-sh/uv#14040 (issue, closed) — uv cannot find resolution for required-environment `sys_platform == 'win32' and platform_machine == 'AMD64'`. Historical rejection of valid Windows AMD64 required-environments in uv 0.7.13. Its linked fix corrected markers inferred from wheels, a different path from the explicit target environment used for pylock installation.
- astral-sh/uv#14041 (pull request, merged) — Fix implied `platform_machine` marker for `win_amd64` platform tag. Merged June 16, 2025, fixing astral-sh/uv#14040 by changing win_amd64 wheel-implied markers from x86_64 to AMD64. The diff does not change target_triple.rs, and the corrected wheel mapping remains in the checkout; this is related precedent, not evidence that the reported target override regressed.
- astral-sh/uv#18327 (pull request, merged) — Add documentation for common marker values. Merged March 5, 2026, closing astral-sh/uv#18300 and documenting AMD64 for Windows x64. Confirms the documented expectation cited by the reporter; it changed documentation and configuration examples, not the target override.

## Supporting evidence

The original source inspection used commit `656d02b614609dd6c982707b6d6c325b6120285b`, the revision linked in the report. The following mapping describes the code before this fix; the working-tree change is detailed below:

- `crates/uv-configuration/src/target_triple.rs:1283`: `Self::Windows | Self::X8664PcWindowsMsvc => "x86_64"`. The adjacent Windows ARM64 branch returns `ARM64`, and the i686 branch returns `x86`.
- `crates/uv-configuration/src/target_triple.rs:1872` and `crates/uv-resolve-operations/src/markers.rs:9`: explicit target selection overrides the interpreter's platform markers using that mapping.
- `crates/uv-pip-commands/src/pylock.rs:114`: pylock installation computes the target environment with `resolution_markers` and passes it to lockfile resolution.
- `crates/uv-lock/src/lock/export/pylock_toml.rs:1266`: installation rejects a lock when none of its declared environments match. Package markers use the same environment at line 1284; that package-selection path is now covered by the updated parent regression.
- `docs/concepts/resolution.md:289`: the common-marker table documents `AMD64` for Windows x64.
- `crates/uv-resolver/src/prioritized_distribution.rs:1040`: `WinAmd64` implies `platform_machine == 'AMD64'`, independently of the explicit target override.
- `CHANGELOG.md:101`: uv 0.13.2, released October 19, 2026, added enforcement of PEP 751 environment restrictions through astral-sh/uv#22594.

## Related-item search context

The existing related-item investigation searched open and closed issues and pull requests for platform markers, Windows target overrides, pylock environment checks, and the exact error. It also inspected linked discussions and fixing diffs. No exact duplicate or previous fix of this x64 target mapping was found.

Related but distinct cases included astral-sh/uv#15060 (32-bit wheel filtering), astral-sh/uv#19308 (native sync), astral-sh/uv#18127 and astral-sh/uv#18163 with astral-sh/uv#18166 (host querying/caching), astral-sh/uv#20396 (rejecting incompatible pylock environments), astral-sh/uv#22596 (exporting environment restrictions), and astral-sh/uv#22099 (Windows ARM emulation).

The documentation chain runs from astral-sh/uv#18298 through astral-sh/uv#18300 to astral-sh/uv#18327. The historical wheel-marker failure links astral-sh/uv#14040 to astral-sh/uv#14041. These concern related marker semantics but do not resolve this target override.

## Fix

Outcome: **fixed** in the working tree.

### Root cause and implementation

`TargetTriple::platform_machine` returned `x86_64` for both `Windows` and `X8664PcWindowsMsvc`. `resolution_markers` supplies this value to the pylock consumer's environment and package filters, and to the platform-specific compiler's resolver before pylock generation. This explains the three demonstrated failures: rejecting a valid environment, skipping a conditional package, and omitting that dependency from the generated lockfile.

The production change is one match-arm value in `crates/uv-configuration/src/target_triple.rs`: both Windows x64 target names now return `AMD64`. The Linux x64, Windows ARM64, and i686 mappings remain unchanged. No changes to serialization, resolution policy, wheel tags, or Python-version handling are needed.

Both parent regression files are updated. The environment test reverses its acceptance/rejection snapshots, the package-marker test requires the installation plan to include `iniconfig`, and the compiler test snapshots the retained package and marker plus its subsequent sync plan. The fixtures and explicit command invocations are retained; no test was deleted or weakened.

Inspection also covered neighboring pylock installation tests, platform-specific and non-universal compilation, pylock sync wheel/tag tests, and the producer/consumer implementations. No additional distinct failure beyond the three parent regressions was identified, so no extra scenarios were added.

### Focused validation

The unchanged parent tests first passed while asserting the bug. After updating expectations and before changing production code, both install tests failed with the reported environment rejection and skipped package; the compile test failed because it still produced `packages = []`. After the one-line production change, all three passed with their corrected snapshots.

Successful debug-profile end-to-end checks used CPython 3.12.15 on Linux:

```sh
cargo test -p uv --locked --test pip_install --test pip_compile --no-fail-fast pep_751_windows_amd64 -- --nocapture
cargo test -p uv --locked --test pip_install --test pip_compile -- pip_compile::python_platform pip_compile::pep_751_compile_non_universal pip_install::pep_751_incompatible_environment pip_install::pep_751_package_requires_python --exact
```

The first command passed all three parent regressions, including the generated pylock consumer round trip. The second passed four neighboring tests covering Linux/Windows target selection, non-universal pylock generation, incompatible Python environments, and package Python requirements. `INSTA_UPDATE=no` ensured comparisons against the explicitly edited expectations.

Windows cross-compilation and Clippy for the changed production crate passed with warnings denied:

```sh
uv tool run --no-config --from cargo-xwin cargo-xwin clippy --target x86_64-pc-windows-msvc -p uv-configuration --locked -- -D warnings
```

This used cargo-xwin 0.23.1, Rust 1.99.0 with the Windows target under `/tmp/uv-22920-rustup`, LLVM 18's bin directory on PATH, and temporary uv, SDK, and Cargo target caches. The check finished successfully in the debug (`dev`) profile.

`cargo fmt --all` and `git diff --check` passed. Formatting used Rust 1.99.0 installed under a temporary `RUSTUP_HOME` because the system toolchain lacked rustfmt and its directory was read-only. The full test suite and release builds were not run. The downstream nab and pipx workflows and a native Windows runtime were not exercised.

Pull request: https://github.com/astral-sh/uv-dev/pull/2996
