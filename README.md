# Regression in uv 0.12.11 with long temp paths on Windows

Issue: astral-sh/uv#21611

Classification: bug

## Summary

The report provides a controlled Windows 11 comparison for
`uv pip install jupyterlab-widgets==3.0.16`: with `LongPathsEnabled=0`, uv 0.12.10
succeeds and uv 0.12.11 repeatedly fails while persisting a temporary file to a deeply nested
path under `site-packages`, ending with OS error 3. Both versions succeed when
`LongPathsEnabled=1`.

The closest repository evidence is astral-sh/uv#21468. It landed between the two reported
releases and changed merged file copies from an intermediate temporary directory followed by
`fs_err::rename` to `copy_atomic_sync`, which creates an adjacent `NamedTempFile` and persists it
to the destination. That is the operation named in the 0.12.11 error. This identifies the leading
change to test, but repository evidence does not yet confirm the Windows root cause.

An older issue, astral-sh/uv#16877, involved the same package and its long nested paths on Windows,
but only in symlink mode and with different OS errors. Its fix, astral-sh/uv#16894, made uv
long-path-aware while explicitly retaining the Windows requirement that `LongPathsEnabled=1`.
Those items are useful history, not a canonical duplicate for this copy/persist regression.

## Draft response

Thanks for the clear version matrix and reproduction. This is a regression rather than a duplicate
of astral-sh/uv#16877: that report concerned symlink mode and different Windows errors, while this
failure occurs while persisting a copied wheel file with `LongPathsEnabled=0`.

astral-sh/uv#21468 landed in 0.12.11 and changed merged wheel copies to use an adjacent temporary
file that is persisted to the destination, matching the operation in this error. That makes it the
leading change to investigate, though the root cause still needs to be confirmed on Windows. The
next step is to run this MRE against that pull request's parent and merge commits with
`LongPathsEnabled=0` and add Windows coverage once confirmed. In the meantime, the report
establishes that 0.12.10 or `LongPathsEnabled=1` avoids the failure.

## Classification

This is a **bug**. Under the same command and Windows registry setting, installation succeeds in uv
0.12.10 and fails in uv 0.12.11. The failure is incorrect installation behavior, and no open issue
or pull request already tracks this specific regression. It should not be classified as a duplicate
of the closed astral-sh/uv#16877 because that issue exercised symlink creation rather than copied
wheel-file persistence and produced OS errors 1314/87 rather than OS error 3.

## Related

- astral-sh/uv#21468 — **Merged pull request, “Avoid per-file temporary directories for merged
  copies.”** This is the strongest release-specific lead. It landed between 0.12.10 and 0.12.11 and
  changed merged copies from copying into a temporary directory followed by `fs_err::rename` to
  `copy_atomic_sync`, which creates an adjacent `NamedTempFile` and persists it to the destination.
  That is the exact operation and error shown by astral-sh/uv#21611, although Windows causality has
  not yet been confirmed.
- astral-sh/uv#16877 — **Closed issue, “Failed to install `jupyterlab-widgets==3.0.16` in Windows via
  symlink.”** It shares the package, platform, and deeply nested destination path, but differs in the
  important mechanics: explicit symlink mode, privilege/parameter errors 1314 and 87, and a fix
  confirmed with long paths enabled. It is adjacent history rather than the same regression.
- astral-sh/uv#16894 — **Merged pull request, “Add a Windows manifest to uv binaries.”** It fixed
  astral-sh/uv#16877 by marking uv as long-path-aware and explicitly documented that the Windows
  `LongPathsEnabled` registry value must also be 1. It does not cover the newly reported behavior
  with that value set to 0.

## Search and supporting evidence

Searches covered open and closed issues plus open, closed, and merged pull requests. Literal terms
included `failed to persist temporary file`, `LongPathsEnabled`, `The system cannot find the path
specified`, `os error 3`, `jupyterlab-widgets`, and the 0.12.10/0.12.11 versions. Conceptual terms
covered Windows `MAX_PATH` and filename limits, long wheel-install destinations, temporary-file
persistence, atomic copy/rename behavior, cache paths, and version-specific regressions and fixes.

The 0.12.10…0.12.11 release comparison contains 53 commits and identifies astral-sh/uv#21468 as
the change directly affecting merged copies. Its diff replaces the older temporary-directory copy
and rename path with `copy_atomic_sync`; the current Windows implementation of that helper reports
the same retry and final persistence messages quoted in astral-sh/uv#21611.

Several plausible results were inspected and ruled out as closer matches. astral-sh/uv#8884 is a
Windows source-build failure caused by uv's long cache/build prefix and was addressed by shortening
that build layout. astral-sh/uv#2410 concerns overly long cache filenames, while
astral-sh/uv#4190 collects older cache/source-build failures resolved by enabling long paths. None
is a final wheel-copy regression introduced in 0.12.11. Reports involving concurrent cache writers
or antivirus locks use different triggers and OS errors and are not the same problem.
