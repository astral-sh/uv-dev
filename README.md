# `--python-platform x86_64-pc-windows-msvc` sets `platform_machine` to `x86_64` instead of `AMD64`

Issue: astral-sh/uv#22920

Classification: **bug**

## Summary

The reporter uses uv 0.13.5 on Linux with Python 3.15.1 to target Windows x64. Both `--python-platform x86_64-pc-windows-msvc` and its `windows` alias supply `platform_machine = "x86_64"`, while the documented Windows x64 value is `AMD64`. A valid `pylock.toml` restricted to `AMD64` is rejected, and package markers requiring `AMD64` evaluate false. The reporter also describes downstream impact on nab-generated locks and pipx's zipapp build; those downstream workflows were not independently verified.

Closest items cover 32-bit target ambiguity, the pylock validation that exposes this mismatch, and earlier Windows wheel-marker fixes/documentation. No exact duplicate was found.

## Reported example

The lockfile contains no packages, isolating environment validation from wheel availability:

```toml
lock-version = "1.0"
environments = ["sys_platform == 'win32' and platform_machine == 'AMD64'"]
requires-python = ">=3.11"
created-by = "test"
packages = []
```

```console
uv pip install --no-config --preview-features pylock --dry-run --target t --python-version 3.11 --python-platform x86_64-pc-windows-msvc -r pylock.toml
error: The target environment is not supported by the lockfile
```

The reporter says replacing `AMD64` with `x86_64` makes the command succeed. Expected behavior is for both Windows x64 target names to evaluate `AMD64` markers successfully.

## Draft response

You're right: both Windows x64 targets currently supply `x86_64` for `platform_machine`, while the documented Windows x64 value is `AMD64`. This explains your pylock environment rejection and the skipped `AMD64`-guarded packages. astral-sh/uv#16576 concerns the separate 32-bit host/target ambiguity; astral-sh/uv#14041 corrected wheel-tag inference but did not change this override. The next step is to correct the target mapping and add regression coverage for your pylock example and conditional dependencies.

## Classification

The source explicitly assigns x86_64 to both Windows x64 target names, contradicting the documented AMD64 marker and causing valid pylock environments and conditional packages to fail marker evaluation. The closest open discussions concern 32-bit host/target ambiguity or wheel filtering, not this x64 override. The historical wheel-marker fix remains present and never changed this mapping, so no regression or exact duplicate was established.

The recent pylock validation in astral-sh/uv#22594 was released in uv 0.13.2 on October 19, 2026, before the reported uv 0.13.5. That change introduced the environment check and error message, exposing the inconsistent target marker. It did not previously correct the target mapping. The older fix in astral-sh/uv#14041 changed wheel-implied markers, and that correction remains present. These histories do not establish a previously fixed target-mapping bug returning.

The 32-bit discussion in astral-sh/uv#16576 asks which host architecture to assume for an i686 target. Changing the x64 spelling to its documented Windows value does not require resolving that separate ambiguity. Likewise, astral-sh/uv#18298 concerns a user-written `x86_64` requirement; here uv generates the inconsistent value itself.

## Related

- astral-sh/uv#16576 (issue, open) — Unexpected dependency resolving with `platform_machine` and `--python-platform`. Closest command-level discussion: a 32-bit Windows target supplies x86 while a 32-bit interpreter on a 64-bit host reports AMD64, changing dependency selection. Maintainers discuss the ambiguous host architecture and require overrides to be host-independent. This does not establish the separate x64 target spelling bug as a duplicate.
- astral-sh/uv#22594 (pull request, merged) — Honor `pylock.toml` environment restrictions during installation. Merged October 15, 2026, and released in uv 0.13.2, adding the exact reported error and evaluation of pylock environments against the target markers. This exposes the incorrect Windows x64 mapping for pylock installs; it neither changes nor fixes that mapping.
- astral-sh/uv#18298 (issue, open) — false positive error installing playwright when `required-environments = ["sys_platform == 'win32' and platform_machine == 'x86_64'"]`. Maintainers confirm AMD64 is the Windows x64 marker expected by wheel compatibility checks. This concerns a user-supplied x86_64 marker in required-environments, whereas the new report concerns uv itself generating that incorrect value through --python-platform.
- astral-sh/uv#14040 (issue, closed) — uv cannot find resolution for required-environment `sys_platform == 'win32' and platform_machine == 'AMD64'`. Historical rejection of valid Windows AMD64 required-environments in uv 0.7.13. Its linked fix corrected markers inferred from wheels, a different path from the explicit target environment used for pylock installation.
- astral-sh/uv#14041 (pull request, merged) — Fix implied `platform_machine` marker for `win_amd64` platform tag. Merged June 16, 2025, fixing astral-sh/uv#14040 by changing win_amd64 wheel-implied markers from x86_64 to AMD64. The diff does not change target_triple.rs, and the corrected wheel mapping remains in the checkout; this is related precedent, not evidence that the reported target override regressed.
- astral-sh/uv#18327 (pull request, merged) — Add documentation for common marker values. Merged March 5, 2026, closing astral-sh/uv#18300 and documenting AMD64 for Windows x64. Confirms the documented expectation cited by the reporter; it changed documentation and configuration examples, not the target override.

## Supporting evidence

Source inspection used checkout commit `656d02b614609dd6c982707b6d6c325b6120285b`, the same revision linked in the report.

- `crates/uv-configuration/src/target_triple.rs:1283`: `Self::Windows | Self::X8664PcWindowsMsvc => "x86_64"`. The adjacent ARM64 Windows branch returns `ARM64`, and the i686 branch returns `x86`.
- `crates/uv-configuration/src/target_triple.rs:1872` and `crates/uv-resolve-operations/src/markers.rs:9`: explicit target selection overrides the interpreter's platform markers using this mapping. The mismatch therefore follows from the target override rather than the Linux host's runtime value.
- `crates/uv-pip-commands/src/pylock.rs:114`: pylock installation computes the target marker environment through `resolution_markers` and passes it to lockfile resolution.
- `crates/uv-lock/src/lock/export/pylock_toml.rs:1266`: installation rejects the lockfile when no declared environment matches. The same environment is used to filter package markers at line 1284. The error text is defined at line 67.
- `docs/concepts/resolution.md:289`: the common-marker table documents `AMD64` for Windows x64. The diff of astral-sh/uv#18327 establishes the documentation history.
- `crates/uv-resolver/src/prioritized_distribution.rs:1040`: `WinAmd64` still implies `platform_machine == 'AMD64'`. The diff of astral-sh/uv#14041 changed this wheel-inference behavior, not `TargetTriple::platform_machine`.
- astral-sh/uv#16576 maintainer comments distinguish host and interpreter architectures and require explicit target values to be independent of the host. astral-sh/uv#18298 maintainer comments recommend `AMD64` and explain that `required-environments` wheel checks affect releases without source distributions.

## Search coverage and exclusions

Searched astral-sh/uv open/closed issues and open/closed/merged PRs with authenticated gh: literal platform_machine/AMD64 and target-triple searches, the exact lockfile error, --python-platform dependency markers, and pylock Windows environments; conceptual searches covered Windows marker compatibility, host/interpreter architecture, target overrides, and area:windows. Keyword PR search omitted known linked PRs, so all 13,295 PR titles were also filtered for related terms. Inspected strongest candidates, comments, timelines, and fix diffs, including the pylock validation and documentation chains. Ruled out astral-sh/uv#15060 (32-bit wheel filtering), astral-sh/uv#19308 (native sync), astral-sh/uv#18127 and astral-sh/uv#18163 with astral-sh/uv#18166 (host querying/caching), astral-sh/uv#20396 and astral-sh/uv#22596 (pylock enforcement/export), and astral-sh/uv#22099 (Windows ARM emulation). No exact duplicate or prior fix of the x64 target mapping was found.

The documentation chain runs from astral-sh/uv#18298 through astral-sh/uv#18300 to astral-sh/uv#18327. The historical wheel-marker failure links astral-sh/uv#14040 to astral-sh/uv#14041. Both fixing diffs were inspected.

The open astral-sh/uv#20396 proposes rejecting genuinely incompatible pylock environments; it does not address incorrect target values. The merged astral-sh/uv#22596 exports environment restrictions, while this report supplies a lockfile directly. The merged astral-sh/uv#22099 retains Windows x64 wheels for ARM-host emulation; it does not change explicit Windows x64 target markers.

Keyword PR searches did not return even known linked pull requests. To compensate, the all-state PR listing was read and all 13,295 titles were filtered for marker, target-platform, Windows, and pylock-environment terms. Relevant candidates were then opened directly, with comments and file changes inspected.

## Validation and next step

The classification is supported by the source path, current documentation, and historical diffs. The reported command was not executed, and no builds or tests were run.

The concrete next step is to correct the Windows x64 target marker and cover both the pylock environment check and conditional dependency selection. Existing `pep_751_incompatible_environment` coverage in `crates/uv/tests/pip_install/pip_install.rs:16105` checks a Python-version mismatch, while `windows_amd64_required` in `crates/uv/tests/lock/lock.rs:40470` covers wheel availability. Neither exercises this explicit Windows x64 override with an `AMD64` marker.
