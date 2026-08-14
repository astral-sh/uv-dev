# `build::venv_included_in_sdist` test fails in 0.12.4 release

Issue: astral-sh/uv#21128

Classification: bug

## Summary

On Gentoo Linux amd64, the `build::venv_included_in_sdist` integration test from uv 0.12.4 fails its second snapshot. Both extraction backends correctly reject the deliberately invalid source distribution because it contains a virtual environment with an absolute Python symlink. The legacy extraction path also prints the intended hint explaining that virtual environments must be excluded from source distributions, but the preview `tar-codec` path omits that hint.

The 0.12.4 source and snapshot explicitly require the hint on both paths. The `tar-codec` handler recognizes a virtual-environment Python target only when its parent directory is named `bin` and its filename starts with `python`. The test environment used in the report resolves the target as `.../python/3.12/python3`, so it does not satisfy that path-shape check. This explains the observed missing hint without changing the validity of the underlying extraction error.

## Draft response

Thanks for the report. The source distribution is still expected to be rejected because it contains a virtual environment, but the tar-codec path should also emit the explanatory hint. In 0.12.4, that hint detection expects the symlink target to look like `bin/python*`; your test interpreter target is instead under `python/3.12/python3`, so the second snapshot loses the hint and fails. This is a regression of the diagnostic added in astral-sh/uv#15202 for astral-sh/uv#15096. We should make the tar-codec detection independent of the test interpreter's installation layout and add coverage for this path shape.

## Classification

This is a bug. The committed 0.12.4 integration test establishes that the tar-codec error path is intended to retain the source-distribution virtual-environment hint. Repository source confirms that the path-shape heuristic does not recognize Gentoo's test interpreter layout, producing incorrect diagnostic output and a snapshot failure.

The behavior restores a platform-sensitive gap in a previously fixed diagnostic: astral-sh/uv#15096 requested the hint, and astral-sh/uv#15202 implemented it. Because no open issue or pull request already tracks this regression, astral-sh/uv#21128 should not be classified as a duplicate.

## Related

- astral-sh/uv#15096 — Closed issue that requested the exact explanatory hint for virtual environments included in source distributions. Its title says “wheels,” but its body and resolution concern the source-distribution extraction error exercised here.
- astral-sh/uv#15202 — Merged pull request that fixed astral-sh/uv#15096 by adding the venv-in-sdist hint and its integration coverage. The new report is a regression of that user-facing diagnostic.
- astral-sh/uv#19979 — Merged pull request that added the preview tar-codec extraction path shortly before the 0.12.4 release. It added structured unsafe-link handling and the `bin/python*` target heuristic that does not match the Gentoo test layout.

## Supporting evidence

- The report's first extraction run includes the expected hint, while the second run reports `unsafe symbolic-link target ... is absolute` without it; only the second snapshot fails.
- The uv 0.12.4 tag was published on 2026-08-13 and contains both the tar-codec handler and a snapshot requiring the hint. astral-sh/uv#19979 merged on 2026-08-11, immediately before that release.
- In the reported environment, the symlink target ends in `python/3.12/python3`. The tar-codec hint logic requires the target's parent to end in `bin`, so it returns no hint for this otherwise equivalent test layout.
- astral-sh/uv#14834 and astral-sh/uv#15086 concern the expected rejection of invalid source distributions containing virtual environments, not this missing-hint test regression. astral-sh/uv#15243 concerns uv_build support for symlinked package directories and is also distinct.

## Search coverage

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included `venv_included_in_sdist`, `unsafe symbolic-link target`, `external symlinks are not allowed`, the complete virtual-environment hint, `Invalid tar file`, Gentoo, and 0.12.4. Conceptual and fix-oriented queries covered source-distribution virtual environments, external symlinks, extraction policy, build hints, structured tar errors, tar-codec, the historical hint issue, and changes merged before the 0.12.4 release. The strongest candidates, their comments, referenced commits, fixing pull requests, source changes, and release timing were inspected.
