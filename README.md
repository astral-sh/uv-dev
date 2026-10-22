# `uv python upgrade` doesn't upgrade free-threaded versions

Issue: astral-sh/uv#22831

Classification: bug

## Summary

Source confirms a free-threaded variant-preservation bug. The closest history is astral-sh/uv#16416 and astral-sh/uv#16420; cleanup is separately tracked by astral-sh/uv#15805 and astral-sh/uv#18770.

The reporter uses uv 0.13.5 on Ubuntu 24.04 under WSL, with Linux x86_64 GNU managed
CPython installations. Both `cpython-3.14.7-linux-x86_64-gnu` and
`cpython-3.14.7+freethreaded-linux-x86_64-gnu` were installed. Running bare
`uv python upgrade` installed standard CPython 3.14.9 but did not install
`cpython-3.14.9+freethreaded-linux-x86_64-gnu`, despite its appearance in
`uv python list --only-downloads`. The expected result is an upgrade of both
installed variants. No error message or full command transcript was supplied.

The reporter also wants to remove old interpreter installations after upgrading,
motivated by updated certificate bundles. That cleanup capability is a separate
request already tracked by astral-sh/uv#15805.

## Draft response

You're right: `uv python upgrade` should upgrade both installed variants. In 0.13.5, the automatic upgrade path drops the free-threaded variant when constructing requests, so it requests only the standard build in your example. The 3.14.9 free-threaded download is present.

For now, explicitly target it with `uv python upgrade 3.14t`. The fix needs to preserve the installed variant and add coverage for upgrading both variants together.

Removing old installations is tracked separately in astral-sh/uv#15805, with an open implementation in astral-sh/uv#18770. Older installations are currently retained because virtual environments may still reference them.

## Classification

Bare uv python upgrade is documented to upgrade all installed managed Python versions. In both the checkout and the 0.13.5 source, converting a managed installation into an upgrade request hardcodes PythonVariant::Default, losing the free-threaded variant and collapsing the reporter's two installations into one standard-CPython request. The requested free-threaded download exists in bundled metadata. The historical implementation fix addresses a different defect; no duplicate or previously fixed instance of this variant bug was found.

Free-threaded CPython is supported functionality, not a request to add a new
interpreter implementation. The documented limitations for transparent upgrades
of PyPy, GraalPy, and Pyodide do not apply to this CPython variant.

The implementation-preservation change in astral-sh/uv#16420 predates this report
and uses the conversion implicated here. Its resolved issue concerns installing
CPython when only PyPy was installed; the current report concerns skipping an
installed CPython variant. There is no evidence of a previously fixed instance of
this exact variant defect returning, and no open canonical duplicate was found.

## Related

- astral-sh/uv#16420 — Check for matching Python implementation during `uv python upgrade` (pull request, merged). Changed bare upgrades to construct requests from managed installations, the same conversion that currently replaces the installed variant with PythonVariant::Default. Merged on 2025-10-23 to fix implementation preservation; its regression test covers PyPy, not free-threaded CPython.
- astral-sh/uv#16416 — `uv python upgrade` installs cpython-3.11 if pypy-3.11 exists (issue, closed). Historical bug in the same automatic upgrade-request construction. Maintainer diagnosis identified lost implementation information; astral-sh/uv#16420 fixed that. This report concerns lost CPython variant information, so it is not a duplicate of the resolved PyPy problem.
- astral-sh/uv#15805 — A way to remove old Python versions (issue, open). Tracks the reporter's secondary cleanup request, including retaining the latest free-threaded versions. Maintainers discuss pruning and existing virtual-environment references. It does not track skipped free-threaded upgrades.
- astral-sh/uv#18770 — Add `--outdated` to `uv python uninstall` and `--uninstall` to `uv python upgrade` (pull request, open). Proposes the cleanup requested in astral-sh/uv#15805, rebasing astral-sh/uv#16018 and adding cleanup after upgrades. It remains unmerged and does not fix the variant loss in automatic upgrade requests.

## Supporting evidence

- `docs/concepts/python-versions.md:174` documents upgrading managed Python to
  the latest supported patch release; line 183 explicitly describes bare
  `uv python upgrade` as upgrading all installed Python versions. Lines
  189–196 explain retaining old installations and explicit patch pins.
- `crates/uv-python-commands/src/install.rs:353` handles an upgrade without
  explicit targets by iterating all managed installations, converting each with
  `PythonDownloadRequest::from(installation)`, dropping the patch and prerelease
  components, and deduplicating the resulting requests in an `IndexSet`.
- `crates/uv-python-managed/src/managed.rs:1090` implements that conversion.
  Line 1099 passes `PythonVariant::Default` rather than the installation's
  variant. The two reported installations therefore produce the same request
  after their patch versions are removed.
- `crates/uv-python-types/src/request.rs:1029` shows that `only_minor()`
  retains the variant it receives. The loss happens before this step.
  `crates/uv-python-types/src/download_request.rs:392` checks the request's
  variant against the installation key.
- The same upgrade loop and hardcoded variant were inspected through the GitHub
  contents API at tag `0.13.5`, confirming applicability to the reported
  release rather than relying only on the checkout.
- `crates/uv-python-managed/download-metadata.json:23649` contains
  `cpython-3.14.9+freethreaded-linux-x86_64-gnu`, with build revision
  `20261021`. The missing upgrade is not explained by absent bundled metadata.
- `CHANGELOG.md:6` dates uv 0.13.5 to 2026-10-21 and describes the CPython
  3.14.9/security build updates. The issue was opened on 2026-10-22.
- astral-sh/uv#13954 introduced Python patch upgrades and explicitly described
  support for standard and free-threaded Python. Its merge on 2025-06-20 closed
  the broader upgrade-all request in astral-sh/uv#12977. The stabilization
  discussion in astral-sh/uv#16637 was subsequently closed as stable.
- Explicit targets take the separate parsing branch in
  `crates/uv-python-commands/src/install.rs:403`. The free-threaded selector
  is documented in `docs/concepts/python-versions.md:333`; explicitly requesting
  `3.14t` avoids reconstructing a request from the installed interpreter.

These findings are based on source, metadata, documentation, and GitHub
discussion review. The reported installation sequence and the workaround were
not executed, and no builds or integration tests were run.

## Existing coverage and next step

`crates/uv/tests/python/python_upgrade.rs` covers bare upgrades across several
minor versions and preservation of the Python implementation, but contains no
free-threaded upgrade scenario. The unit test
`upgrade_request_preserves_variant` in
`crates/uv-python-types/src/download_request.rs:957` starts with an already
correct free-threaded request and tests display transformations; it does not
exercise conversion from a managed installation.

A focused fix should retain the installed variant when constructing the download
request. Add an integration scenario in the existing Python upgrade test module
that installs older standard and free-threaded patches, invokes bare upgrade,
and snapshots both upgraded variants. Follow the module's existing fixture,
snapshot, and feature-gating conventions. No reproduction details are needed to
establish the classification.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using literal python upgrade/free-threaded/freethreaded, 3.14, 3.14t, and 3.14.9+freethreaded terms; conceptual update, nogil/no-GIL, variant, skip, and managed-interpreter terms; and separate old-version removal, outdated, and prune searches. Searched PythonDownloadRequest and variant-preservation identifiers separately from symptoms. PR search returned no matches, so additionally enumerated all 13,224 PR titles and inspected relevant bodies, comments, reviews, and diffs. Followed historical fix and cleanup references. Ruled out astral-sh/uv#20434 (explicit equality parsing), astral-sh/uv#18795 and astral-sh/uv#19012 (Windows registry listing), astral-sh/uv#21323 (replacement of same-version build revisions), and astral-sh/uv#16675 (non-CPython transparent upgrades). No existing tracker for this exact defect or prior fix of it was found.

Additional inspected candidates were excluded for these reasons:

- astral-sh/uv#18862 concerns reusing a free-threaded environment for an
  unconstrained `3.14` interpreter request. A maintainer explains that
  `3.14+gil` requests the GIL-enabled interpreter. This does not concern
  downloading patch upgrades.
- astral-sh/uv#8189 concerns whether an explicit uninstall request for
  `3.13` should also remove `3.13t`; it is distinct from upgrading every
  installed variant without targets.
- astral-sh/uv#21790 and astral-sh/uv#21794 concern provider-defined and
  optimization build identities. Both were closed without merging in favor of
  another approach; they do not establish a fix for this free-threaded upgrade
  defect.
- astral-sh/uv#15756 discusses documentation and implicit free-threaded
  interpreter selection, not automatic patch upgrade requests.
- astral-sh/uv#16783 concerns an ambiguous message when no managed installations
  are available, not skipping one of two installed variants.
- astral-sh/uv#16018 is the earlier, still-open cleanup proposal. The inspected
  successor astral-sh/uv#18770 rebases it and adds cleanup after upgrades; neither
  resolves variant loss during automatic request construction.

No checkout files were modified and no GitHub changes were made. This document
and the proposed response are for maintainer review.
