# 0.12.14: `uv pip install --system` fails in official `python:*` Docker images — "Cannot install into symlinked directory: /usr/local/man"

Issue: astral-sh/uv#21692

Classification: bug

## Summary

The reported regression is reproducible. On x86_64 Linux, uv 0.12.14 fails to install `clevercsv==0.8.4` into the current official `python:3.11-slim-bookworm` image because the wheel's `.data/data` tree contains `man`, while the image has `/usr/local/man -> share/man`. The same installation succeeds with uv 0.12.13 in the identical Python image. This independently confirms the report outside its macOS/arm64 host and shows that the behavior is not specific to the reporter's architecture or uv's musl build.

Repository evidence is consistent with the observed version boundary. astral-sh/uv#21569 added `ValidatedWheelDestination` and the exact error, including validation of a wheel's `.data/data` subtree against the installation scheme's data root. It merged after the 0.12.13 release and is present in 0.12.14. The validation rejects any existing destination directory symlink encountered for a wheel directory, without resolving the link and checking whether it remains inside the trusted installation prefix. The 0.12.14 release notes do not list astral-sh/uv#21569.

No pre-existing issue or pull request was found that already tracked this specific 0.12.14 regression. A subsequent implementation is recorded in astral-sh/uv-dev#1806, and maintainer konstin confirmed that the fix was released in uv 0.12.15.

## Reproduction

Outcome: **reproducible**.

The installed uv executable on `PATH` was mounted read-only into a fresh container and given a container-local cache:

```console
$ uv --version
uv 0.12.14 (x86_64-unknown-linux-gnu)
$ docker run --rm \
    --volume /opt/hostedtoolcache/uv/0.12.14/x86_64/uv:/tmp/uv:ro \
    python:3.11-slim-bookworm \
    sh -c 'python --version && uname -m && ls -ld /usr/local/man /usr/local/share/man && /tmp/uv --version && UV_CACHE_DIR=/tmp/uv-cache /tmp/uv pip install --system clevercsv==0.8.4'
Python 3.11.16
x86_64
lrwxrwxrwx 1 root root    9 Aug 24 00:00 /usr/local/man -> share/man
drwxr-xr-x 1 root root 4096 Sep  1 00:16 /usr/local/share/man
uv 0.12.14 (x86_64-unknown-linux-gnu)
Using Python 3.11.16 environment at: /usr/local
Resolved 4 packages in [TIME]
Prepared 3 packages in [TIME]
error: Failed to install: clevercsv-0.8.4-cp311-cp311-manylinux1_x86_64.manylinux_2_28_x86_64.manylinux_2_5_x86_64.whl (clevercsv==0.8.4)
  cause: The wheel is invalid: Cannot install into symlinked directory: /usr/local/man
```

The image digest used was `python:3.11-slim-bookworm@sha256:528257d48c1da0dcecc2e725d1ae34498d60c965f1241e39cd6a85a8859bdf84`. The command exited with status 2.

For the control, the uv 0.12.13 binary was extracted into a temporary directory from `ghcr.io/astral-sh/uv:0.12.13@sha256:b485bd65cc2cf1c9a93b3554012c9c3778cf7b1b5fd3d3096ce9e1226c97e1e6` and mounted into the same Python image. With the same Python version, architecture, symlink, package version, and command, it exited successfully:

```console
uv 0.12.13 (x86_64-unknown-linux-musl)
Using Python 3.11.16 environment at: /usr/local
Resolved 4 packages in [TIME]
Prepared 3 packages in [TIME]
Installed 3 packages in [TIME]
 + chardet==7.6.0
 + clevercsv==0.8.4
 + regex==2026.9.10
```

Existing integration coverage in `crates/uv/tests/pip_install/pip_install.rs` includes `reject_symlinked_wheel_package_directory`, `reject_symlinked_wheel_nested_package_directory`, `reject_symlinked_wheel_data_package_directory`, and `reject_symlinked_wheel_headers_destination`. Those tests construct destination symlinks to external temporary directories and verify that uv rejects the installation before writing through them. They do not cover a scheme directory such as `/usr/local/man -> share/man` whose resolved target remains inside the same installation prefix.

## Fix

Outcome: **fixed**.

Maintainer konstin confirmed in astral-sh/uv#21692 that the regression is fixed in uv 0.12.15. The associated implementation is recorded in astral-sh/uv-dev#1806.

The root cause was the unconditional rejection of every existing destination-directory symlink encountered while validating a wheel subtree. The validation already has a trusted installation root, but it inspected only the destination's file type and never resolved the link to determine whether it crossed that boundary.

`crates/uv-install-wheel/src/wheel.rs` now canonicalizes both an encountered symlink and its installation root. It permits the symlink only when the resolved destination remains within that canonical root. Links that resolve outside the root, and links whose destinations cannot be resolved, continue to produce the existing invalid-wheel error.

The parent regression in `crates/uv/tests/pip_install/pip_install.rs` now expects the `.data/data/man` wheel to install successfully through `man -> share/man`, checks the installed manual-page contents, uninstalls the distribution, and verifies that the data file is removed. No separate manifestation warranted another test: purelib, platlib, data, package, nested-package, and headers destinations all use the same `ValidatedWheelDestination` consumer, while the adjacent uninstall path is covered by the round trip in the updated parent test.

Focused debug-profile validation passed for the updated install/uninstall regression and for all five neighboring `symlinked_wheel` integration tests. The latter retain coverage that package, nested-package, wheel-data, and headers symlinks pointing to external directories are rejected. `cargo fmt` and focused Clippy validation for `uv-install-wheel` also passed with the repository's Rust 1.98.1 stable toolchain.

## Draft response

Thanks for the detailed reproduction. astral-sh/uv#21569 introduced the destination-symlink check in uv 0.12.14 to prevent a wheel payload from being written outside its installation environment. The check also rejected `/usr/local/man` without considering that its relative target, `/usr/local/share/man`, remains within the same installation prefix, so this was a regression rather than an invalid wheel.

The validation now resolves destination links and allows them only when their targets remain within the installation root. Regression coverage verifies installation and uninstallation through the standard in-prefix layout while retaining the existing rejection tests for links that escape the environment.

## Classification

This is a **bug**, not a duplicate. The version-controlled reproduction establishes a regression in uv 0.12.14 on a standard official Python image. The current source shows that uv unconditionally rejects a pre-existing directory symlink encountered beneath a wheel destination. Here `/usr/local/man` resolves to `/usr/local/share/man`, still within `/usr/local`, and uv 0.12.13 installs the same wheel successfully. The user-facing `The wheel is invalid` cause is also misleading because the rejection is determined by the destination layout, not malformed wheel contents.

astral-sh/uv#21569 is the causative historical change, but it did not track this regression and therefore was not a canonical duplicate. No pre-existing issue or pull request tracked the regression when it was reported; the subsequent fix is recorded in astral-sh/uv-dev#1806.

## Related

- astral-sh/uv#21569 — **merged pull request, “Reject symlinked wheel installation destinations.”** This is the closest result and the direct source of the behavior. It added the exact error and validates `.data/data` destinations to prevent installation through directory symlinks that could redirect writes outside the environment. Its implementation does not distinguish `/usr/local/man -> share/man`, whose resolved target remains under the installation prefix. It merged on 2026-09-10 after uv 0.12.13 was released and shipped in uv 0.12.14 on 2026-09-15.
- astral-sh/uv-dev#1806 — **open implementation pull request, “Allow wheel installation through in-prefix directory symlinks.”** It records the fix that permits resolved destinations within the installation root while retaining rejection of escaping or unresolvable links. Although this pull request remains open, a maintainer confirmed the fix is included in uv 0.12.15.
- astral-sh/uv#21255 — **open issue, “uv sync and uv pip install fails for packages with console scripts when the venv contains a `lib` to `usr/lib` symlink.”** This is adjacent symlink-related installer work, but not the same problem. It predates uv 0.12.14, uses a synthetic venv merged-`/usr` layout, and concerns relative console-script path calculation that ends in a metadata lookup failure. Its open proposed fix, astral-sh/uv#21256, normalizes or resolves script paths; it does not address `.data/data` destination validation.

## Search and supporting evidence

Initial searches covered open and closed issues and open, closed-unmerged, and merged pull requests. Literal searches used the exact error, `/usr/local/man`, `share/man`, `.data/data`, and `uv pip install --system`. Conceptual searches covered wheel data files, mapped installation destinations, symlinked scheme roots, links that remain within a prefix versus links that escape an environment, official Python Docker layouts, and man pages. No pre-existing tracker was found; astral-sh/uv-dev#1806 was created subsequently to record the fix.

The strongest ruled-out candidates were:

- astral-sh/uv#15243, which concerns `uv_build` traversing directory symlinks in a package source tree, a different subsystem and direction of operation.
- astral-sh/uv#4731 and astral-sh/uv#11354, which request ways to expose man pages and shell completions installed inside `uv tool` environments; they do not concern system-scheme wheel data installation.
- astral-sh/uv#18942, which protects uninstall operations from malicious `RECORD` entries outside an environment. It is part of the broader environment-boundary work referenced by astral-sh/uv#21569, but it does not produce or track this install-time regression.

Current tests added by astral-sh/uv#21569 cover rejecting symlinked package, nested package, data-package, and headers destinations when those links point to external directories. They do not cover a standard installation-scheme directory symlink whose resolved target remains within the same trusted prefix.
