# test fixture test/packages/fake-uv/src is a committed symlink that breaks on an unprivileged Windows checkout

Issue: astral-sh/uv#21850

Classification: bug

## Summary

The report identifies `test/packages/fake-uv/src` as a committed symbolic link that Git materializes as a plain target-text file on Windows checkouts where `core.symlinks=false`. The `fake-uv` package then lacks the source tree expected by the `python_module` integration tests.

Repository evidence supports the report's central claim: the entry is the repository's sole mode-`120000` file and targets `../../../python/`; the fixture README says it links uv's in-tree Python module; and `crates/uv/tests/python/python_module.rs` repeatedly installs the fixture. The repository also already treats Windows symlink availability as conditional in other tests. A plain file at `src` cannot fulfill this fixture design.

No existing issue or pull request was found that tracks the checkout defect. astral-sh/uv#15110 introduced the symlink-based fixture at its former `scripts/packages/fake-uv/src` path, and astral-sh/uv#17032 moved it to the current path while preserving the link.

## Draft response

Thanks for the detailed report. The repository confirms that `test/packages/fake-uv/src` is a committed symbolic link and that the `python_module` integration tests install this fixture. With `core.symlinks=false`, materializing that entry as a plain file cannot provide the source tree the fixture expects, so this is a test-fixture bug. A fix should remove checkout-time symlink support as a prerequisite for these tests; whether to package the needed files directly or construct the fixture during test setup needs maintainer review.

## Classification

This is a bug. The fixture is intended to expose the in-tree Python package as the source of a mock `uv` distribution, but under the reported Windows checkout condition it instead contains a regular file named `src`. That makes repository integration tests unusable even though the checkout behavior is expected when Git cannot create symbolic links.

The committed mode, link target, fixture documentation, and test consumers are source-confirmed. The reporter's specific Windows reproduction was not rerun in this Unix checkout, but the repository's own tests note that Windows does not allow symbolic links by default or may require elevated privilege. No existing tracker covers this exact failure, so the issue is not a duplicate.

## Related

- astral-sh/uv#15110 (merged pull request, “Add test cases for `find_uv_bin`”) introduced the `fake-uv` package, its mode-`120000` `src` link, and the integration tests that consume it. Its description explicitly says the link reuses uv's in-tree Python module without building or packaging the binary. It establishes the design behind the failure but does not discuss Windows checkout behavior.
- astral-sh/uv#17032 (merged pull request, “Move test support files out of `scripts/` into `test/`”) renamed the fixture to its current location, preserved the symbolic link, and updated the integration tests to use `test/packages/fake-uv`. It is relevant path history, not an existing report or fix.

## Search coverage and supporting evidence

Searches covered this repository's open and closed issues and open, closed, and merged pull requests. Literal queries included the exact current and former fixture paths, `fake-uv`, `uv==0.1.0`, `core.symlinks`, `Developer Mode`, and the plain target-text-file symptom. Conceptual queries covered unprivileged Windows checkouts, privilege-dependent symbolic links, malformed test fixtures, local `python_module` failures, and copy, materialization, or documentation alternatives. No prior tracker or fix for this defect was found.

Remote path history confirms that astral-sh/uv#15110 created the link and astral-sh/uv#17032 moved it unchanged. The current tree contains no other committed symbolic link, while the integration test source has multiple installs from `test/packages/fake-uv`.

The strongest superficially similar issue was astral-sh/uv#15368, which also reported local failures in the `python_module` tests using `fake-uv`. Its output showed the tests finding `/usr/bin/uv`, and astral-sh/uv#15611 fixed that by overriding `sys.base_prefix`; it did not involve checkout materialization. astral-sh/uv#15188 was another installed-system-uv test regression with the same important difference. astral-sh/uv#6782 concerns copy semantics for virtual-environment interpreters, not repository fixtures, and astral-sh/uv#4374 concerned version strings in snapshots from unpacked sources rather than symbolic links.
