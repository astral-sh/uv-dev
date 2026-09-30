# UV builds fine with NetBSD but refuses to install

Issue: astral-sh/uv#22110

Classification: bug

## Summary

On NetBSD 11_STABLE x86_64 with Python 3.14.7 and uv 0.12.21, maturin 1.15.0 successfully builds native wheels for packages including pyreqwest, orjson, and ty, but uv then rejects the wheels as incompatible. The reported wheel tag is `cp314-cp314-netbsd_11_0_STABLE_amd64`; the reporter also demonstrated that an extracted orjson extension loads on the same host.

The behavior is reproducible with a minimal local PEP 517 backend and an emulated NetBSD interpreter probe. uv 0.12.21 rejects a built wheel whose NetBSD release component contains uppercase `STABLE`, but accepts an otherwise identical wheel with lowercase `stable`. uv 0.12.17 accepts the uppercase wheel, placing the regression boundary at uv 0.12.18, which included astral-sh/uv#21853.

## Reproduction

Outcome: **reproducible**.

The runner is Linux x86_64, so the reproduction used uv's normal interpreter-discovery interface to report the relevant NetBSD platform (`sysconfig.get_platform() == "netbsd-11.0_STABLE-amd64"`) while delegating builds to the host CPython 3.12.3. A minimal local PEP 517 backend, with no build dependencies, returned one of these wheel filenames:

```text
casewheel-1.0.0-cp312-cp312-netbsd_11_0_STABLE_amd64.whl
casewheel-1.0.0-cp312-cp312-netbsd_11_0_stable_amd64.whl
```

The core command was run using the installed `uv` as the `uvx` launcher so the reported release could be tested exactly; all files, targets, tools, Python data, and caches were under `/tmp/uv-issue-22110-repro`:

```console
$ uvx --from uv==0.12.21 uv pip install /tmp/uv-issue-22110-repro/uppercase \
    --target /tmp/uv-issue-22110-repro/target-uppercase \
    --python /tmp/uv-issue-22110-repro/netbsd-python \
    --cache-dir /tmp/uv-issue-22110-repro/inner-cache-uppercase \
    --no-config --no-progress
Resolved 1 package in 1ms
      Built casewheel @ file:///tmp/uv-issue-22110-repro/uppercase
error: Failed to build `casewheel @ file:///tmp/uv-issue-22110-repro/uppercase`
  cause: The built wheel `casewheel-1.0.0-cp312-cp312-netbsd_11_0_STABLE_amd64.whl` is not compatible with the current Python 3.12 on NetBSD x86_64
```

Changing only `STABLE` to `stable` made uv 0.12.21 prepare and install `casewheel==1.0.0`. Running the uppercase fixture with uv 0.12.17 also prepared and installed it successfully. This isolates the failure to platform-tag case handling; it does not depend on Python 3.14, maturin, or a particular native extension.

The observed release boundary matches the implementation. astral-sh/uv#21853 changed `crates/uv-platform-tags/src/tags.rs` so a detected NetBSD release such as `11.0_STABLE` generates the compatible tag `netbsd_11_0_stable_amd64`. `PlatformTag::from_str` in `crates/uv-platform-tags/src/platform_tag.rs` retains the `ReleaseArch` suffix's original case when parsing a wheel filename, so the uppercase built-wheel tag and lowercase compatible tag compare unequal.

Before the parent regression pull request, existing tests did not cover this case. `crates/uv-platform-tags/src/tags.rs`, test `test_platform_tags_bsd`, asserted that `11.0_STABLE` generates `netbsd_11_0_stable_amd64`, but did not compare that tag with an uppercase parsed wheel tag. `crates/uv/tests/pip_install/pip_install.rs`, test `build_backend_wrong_wheel_platform`, verified rejection of genuinely incompatible built-wheel Python tags and the associated error path, but did not exercise NetBSD or case normalization. The parent pull request added `build_backend_netbsd_uppercase_release` to cover the missing end-to-end case.

## Workaround

The reporter confirmed that forcing maturin to emit a lowercase NetBSD release tag avoids the failure:

```console
MATURIN_NETBSD_VERSION=11.0_stable uv sync
```

This is a temporary workaround for affected maturin builds: it changes the built wheel's platform tag from `netbsd_11_0_STABLE_amd64` to the lowercase form accepted by uv 0.12.21. It does not address uv's case-preserving wheel-tag parsing.

## Classification

This is a reproducible regression and a bug. A wheel built for the active NetBSD interpreter is rejected solely because its valid platform tag retains uppercase characters from the release name. The same uppercase fixture installs with uv 0.12.17, and the equivalent lowercase fixture installs with uv 0.12.21.

## Fix

Outcome: **fixed**.

`ReleaseArch::from_str` now canonicalizes valid ASCII release-and-architecture suffixes to lowercase. This applies the same normalization to parsed BSD-style wheel tags that compatible-tag generation already applies to detected releases, so `netbsd_11_0_STABLE_amd64` compares equal to the active interpreter's `netbsd_11_0_stable_amd64` tag. Invalid characters remain rejected as before.

The parent integration test in `crates/uv/tests/pip_install/pip_install.rs`, `build_backend_netbsd_uppercase_release`, was updated from snapshotting the rejection to requiring a successful build and installation. The existing `ReleaseArch` unit test now verifies uppercase-to-lowercase canonicalization. No separate command or configuration manifestation was added: inspected install, expanded-tag, and lock consumers all use the same `PlatformTag` parser, while the nearby FreeBSD lock fixture exercises Python-version coverage rather than host-platform compatibility.

Focused debug-profile validation passed for `platform_tag::tests::release_arch`, `tags::tests::test_platform_tags_bsd`, `pip_install::build_backend_netbsd_uppercase_release`, and the neighboring `pip_install::build_backend_wrong_wheel_platform`. A focused `cargo +stable clippy --package uv-platform-tags --all-targets -- -D warnings` check and repository-wide Rust formatting also passed.

## Related

- astral-sh/uv#21846 — Closed issue reporting the original casing mismatch on NetBSD 11 and Python 3.14. It involved a lowercase wheel tag compared with uv's then-uppercase generated compatible tag.
- astral-sh/uv#21853 — Merged pull request that fixed astral-sh/uv#21846 by lowercasing generated NetBSD and other BSD-like platform tags. It shipped in uv 0.12.18 and introduced the demonstrated inverse mismatch for uppercase wheel tags.

Pull request: astral-sh/uv-dev#2231
