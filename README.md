# test fixture test/packages/fake-uv/src is a committed symlink that breaks on an unprivileged Windows checkout

Issue: astral-sh/uv#21850

Classification: bug

## Summary

The reported behavior is reproducible. `test/packages/fake-uv/src` is a committed symbolic link that Git materializes as a plain target-text file in a checkout where `core.symlinks=false`, matching the reported unprivileged Windows checkout behavior. The resulting `fake-uv` package lacks the source tree expected by the `python_module` integration tests and cannot be built.

The entry is the repository's sole mode-`120000` file and targets `../../../python/`. The fixture README says it links uv's in-tree Python module, and the tests in `crates/uv/tests/python/python_module.rs` repeatedly install the fixture. The repository also treats Windows symlink availability as conditional in other tests.

No existing issue or pull request was found that tracks the checkout defect. astral-sh/uv#15110 introduced the symlink-based fixture at its former `scripts/packages/fake-uv/src` path, and astral-sh/uv#17032 moved it to the current path while preserving the link.

## Reproduction

Outcome: **reproducible**.

The reproduction used repository commit `7b090fba99bc89a6670a23de5368d48d2418a756`, Git 2.55.0, uv 0.12.13 (`x86_64-unknown-linux-gnu`), and CPython 3.12.3. Although the runner was Linux rather than Windows, configuring the temporary worktree with `core.symlinks=false` exercises Git's reported checkout materialization directly:

```console
$ git clone --no-checkout --no-local /home/runner/work/uv/uv /tmp/uv-21850/checkout
$ git -C /tmp/uv-21850/checkout config core.symlinks false
$ git -C /tmp/uv-21850/checkout checkout HEAD
$ git -C /tmp/uv-21850/checkout config --get core.symlinks
false
$ stat -c '%F, %s bytes' /tmp/uv-21850/checkout/test/packages/fake-uv/src
regular file, 16 bytes
$ sed -n '1p' /tmp/uv-21850/checkout/test/packages/fake-uv/src
../../../python/
$ git -C /tmp/uv-21850/checkout ls-files -s test/packages/fake-uv/src
120000 e22ace59e47a6957bd8c615cd58f11c40d3133d2 0 test/packages/fake-uv/src
```

Installing the materialized fixture with the installed uv executable and an isolated virtual environment and cache failed as expected:

```console
$ uv pip install --python /tmp/uv-21850/venv/bin/python /tmp/uv-21850/checkout/test/packages/fake-uv
Building uv @ file:///tmp/uv-21850/checkout/test/packages/fake-uv
Failed to build `uv @ file:///tmp/uv-21850/checkout/test/packages/fake-uv`
Expected a Python module at:
/tmp/uv-21850/checkout/test/packages/fake-uv/src/uv/__init__.py
```

The command exited with status 1. As a control, the same installation from the original symlink-preserving checkout succeeded, installed `uv==0.1.0`, and produced `site-packages/uv/__init__.py`.

Existing coverage in `crates/uv/tests/python/python_module.rs` includes 14 `find_uv_bin_*` integration tests that rely on installing this fixture, but none constructs a checkout with `core.symlinks=false`. Those tests cover the fixture's intended behavior when `src` is a working link, not its materialization as a regular file.

## Fix

Outcome: **fixed**.

The failure is confined to how the integration tests consume the `fake-uv` fixture; uv's package builder should not interpret an arbitrary text file as a symbolic link. `TestContext::materialize_fake_uv` now constructs a temporary package from the fixture's `pyproject.toml` and fake script while copying the in-tree `python` directory into `src`. Every consumer in `crates/uv/tests/python/python_module.rs` installs that materialized package instead of relying on the checkout-time symlink.

The parent regression still constructs the `core.symlinks=false` form, including the regular `src` file containing the link target, but now passes it through the same materializer and snapshots a successful installation. This verifies the reported checkout state without requiring symlink privileges. The neighboring `find_uv_bin_*` tests were updated to exercise the same producer/consumer path and retain their behavioral assertions.

The repository contains no other committed symbolic links. Related coverage in `crates/uv/tests/build/build_backend.rs` and `crates/uv/tests/pip/pip_sync.rs` creates symlinks at runtime and is Unix-gated where symlink support is required, so it does not share this checkout-materialization failure and required no change.

Focused validation passed for the parent regression, all 15 `python_module` tests, native Clippy for the Python integration-test target, Windows cross-Clippy for the same target, Rust formatting, and diff whitespace checks.

## Draft response

Thanks for the detailed report. We reproduced the failure by checking out the repository with `core.symlinks=false`: Git created a regular 16-byte `src` file containing `../../../python/`, and uv then failed to build the fixture because `src/uv/__init__.py` was absent. The integration tests now materialize `fake-uv` in their temporary directory with the in-tree Python module copied into `src`, so they no longer require checkout-time symlink support. The regression test recreates the affected checkout form and confirms that the materialized package installs successfully.

## Classification

This is a bug. The fixture is intended to expose the in-tree Python package as the source of a mock `uv` distribution, but with `core.symlinks=false` it instead contains a regular file named `src`. uv cannot build the fixture in that state, so integration tests that install it cannot run successfully.

The Windows host condition itself was not rerun on this Linux runner, but the reported Git materialization and resulting package failure were both observed in a temporary checkout. The repository's own tests note that Windows does not allow symbolic links by default or may require elevated privilege. No existing tracker covers this exact failure, so the issue is not a duplicate.

## Related

- astral-sh/uv#15110 (merged pull request, “Add test cases for `find_uv_bin`”) introduced the `fake-uv` package, its mode-`120000` `src` link, and the integration tests that consume it. Its description explicitly says the link reuses uv's in-tree Python module without building or packaging the binary. It establishes the design behind the failure but does not discuss Windows checkout behavior.
- astral-sh/uv#17032 (merged pull request, “Move test support files out of `scripts/` into `test/`”) renamed the fixture to its current location, preserved the symbolic link, and updated the integration tests to use `test/packages/fake-uv`. It is relevant path history, not an existing report or fix.

## Search coverage and supporting evidence

Searches covered this repository's open and closed issues and open, closed, and merged pull requests. Literal queries included the exact current and former fixture paths, `fake-uv`, `uv==0.1.0`, `core.symlinks`, `Developer Mode`, and the plain target-text-file symptom. Conceptual queries covered unprivileged Windows checkouts, privilege-dependent symbolic links, malformed test fixtures, local `python_module` failures, and copy, materialization, or documentation alternatives. No prior tracker or fix for this defect was found.

Remote path history confirms that astral-sh/uv#15110 created the link and astral-sh/uv#17032 moved it unchanged. The current tree contains no other committed symbolic link, while the integration test source has multiple installs from `test/packages/fake-uv`.

The strongest superficially similar issue was astral-sh/uv#15368, which also reported local failures in the `python_module` tests using `fake-uv`. Its output showed the tests finding `/usr/bin/uv`, and astral-sh/uv#15611 fixed that by overriding `sys.base_prefix`; it did not involve checkout materialization. astral-sh/uv#15188 was another installed-system-uv test regression with the same important difference. astral-sh/uv#6782 concerns copy semantics for virtual-environment interpreters, not repository fixtures, and astral-sh/uv#4374 concerned version strings in snapshots from unpacked sources rather than symbolic links.

Pull request: https://github.com/astral-sh/uv-dev/pull/1991
