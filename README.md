# uv rejects a wheel package built locally, because it uses case-sensitive compare for NetBSD tag

Issue: astral-sh/uv#21846

Classification: bug

## Summary

The reported rejection is reproducible with uv 0.12.13. A valid local wheel tagged
`netbsd_11_0_stable_amd64` is rejected when the interpreter reports NetBSD release
`11.0-STABLE`; uv generates the current-platform tag `netbsd_11_0_STABLE_amd64` and reports the two
tags as incompatible. An otherwise identical wheel whose filename uses uppercase `STABLE` installs
successfully.

The checkout's compatible-tag construction replaces dots and hyphens in FreeBSD and NetBSD
releases, but only the FreeBSD branch lowercases the release. This agrees with the observed tags,
although the reproduction—not source inspection—is the basis for the outcome.

## Reproduction

Outcome: **reproducible**.

The reproduction used the installed `uv 0.12.13 (x86_64-unknown-linux-gnu)` on Ubuntu 24.04.5 with
CPython 3.12.3. Because the runner is not NetBSD, a controlled Python executable shim changed only
uv's interpreter-query response to the relevant NetBSD values: `sysconfig.get_platform()` returned
`netbsd-11.0-STABLE-amd64`, and the platform markers reported NetBSD 11.0-STABLE on amd64. A generic
`py3-none` wheel was used so the Python 3.12 versus reported Python 3.14.6 difference could not
affect compatibility. All fixture, target, and cache paths were under `$RUNNER_TEMP`.

The minimal failing command was equivalent to:

```console
$ UV_CACHE_DIR=$RUNNER_TEMP/uv-21846/cache uv pip install \
    --no-index \
    --python $RUNNER_TEMP/uv-21846/bin/python-netbsd \
    --target $RUNNER_TEMP/uv-21846/target \
    $RUNNER_TEMP/uv-21846/netbsd_case-1.0.0-py3-none-netbsd_11_0_stable_amd64.whl
Using CPython 3.12.3 interpreter at: /usr/bin/python3
Resolved 1 package in 1ms
error: Failed to determine installation plan
  Caused by: A path (.../netbsd_case-1.0.0-py3-none-netbsd_11_0_stable_amd64.whl) dependency is incompatible with the current platform

hint: The wheel is compatible with NetBSD (`netbsd_11_0_stable_amd64`), but you're on NetBSD (`netbsd_11_0_STABLE_amd64`)
```

As a control, the same wheel archive renamed to
`netbsd_case-1.0.0-py3-none-netbsd_11_0_STABLE_amd64.whl` installed successfully with the same shim:

```console
Resolved 1 package in 1ms
Prepared 1 package in 2ms
Installed 1 package in 0.57ms
 + netbsd-case==1.0.0
```

The reproduction therefore observes the reported case-sensitive incompatibility independently of
the package's maturin payload or CPython ABI. It does not test the native NetBSD uv binary, but it
exercises uv 0.12.13's normal interpreter query, compatible-tag generation, installation-plan, and
wheel-selection paths with the reported NetBSD platform data.

No existing integration test under `crates/uv/tests/` or `crates/uv-client/tests/it/` covers NetBSD
wheel installation or NetBSD compatible-tag generation. The only NetBSD-specific test assertion is
`crates/uv-platform-tags/src/platform_tag.rs::invalid_characters_platform`, which verifies that a
NetBSD tag containing a backslash is rejected; it does not cover release casing or compatibility.
`crates/uv-platform-tags/src/tags.rs::test_platform_tags_invalid_release_arch` covers invalid
FreeBSD input only.

## Draft response

Thanks, this is reproducible as a bug in uv 0.12.13. With NetBSD release `11.0-STABLE`, uv rejects a
local wheel tagged `netbsd_11_0_stable_amd64` and generates
`netbsd_11_0_STABLE_amd64` as the current-platform tag. The same fixture installs when the filename
uses uppercase `STABLE`. The current compatible-tag code lowercases FreeBSD releases but not NetBSD
releases, and there is no NetBSD casing regression test. The next step is to normalize the NetBSD
release consistently and add focused compatible-tag and installation coverage.

## Classification

This is a correctness bug: uv treats NetBSD platform tags that differ only in release-letter case as
incompatible. The result is not inferred from the related FreeBSD report; it was reproduced through
the installation path, including a successful uppercase control. This is not a duplicate of
astral-sh/uv#15799 or astral-sh/uv#15829, which addressed FreeBSD. It is also not a regression of
that change: the NetBSD lowercasing behavior was not added there.

## Related

- astral-sh/uv#15799 — Closed issue reporting the FreeBSD analogue: a lowercase locally built wheel
  tag was rejected against an uppercase generated release tag.
- astral-sh/uv#15829 — Merged pull request that fixed astral-sh/uv#15799 by lowercasing the FreeBSD
  release and using BSD-native architecture names. It handled NetBSD architecture naming but did
  not lowercase the NetBSD release.

## Search and evidence

Searches covered NetBSD tag parsing and generation, the installation-plan compatibility hint,
integration tests in `crates/uv/tests/` and `crates/uv-client/tests/it/`, and BSD-related repository
history. astral-sh/uv#3824 and astral-sh/uv#13713 concern libc or interpreter operating-system
detection rather than wheel platform-tag normalization. No separate NetBSD-specific tracker or
NetBSD tag-fix pull request was identified beyond astral-sh/uv#21846.
