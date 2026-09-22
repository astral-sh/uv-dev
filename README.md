# "RECORD file is invalid" recurrent error on Windows

Issue: astral-sh/uv#21905

Classification: duplicate

## Summary

On Windows 11 with uv 0.11.32, creating a fresh environment intermittently fails while installing
`colorlog`, reporting that its `RECORD` is invalid because another process locked part of the file
(os error 33). The failure affects developer machines and Jenkins nodes, usually clears after several
retries, and occurs where antivirus software is active and cannot be disabled.

Open astral-sh/uv#2810 is the canonical tracker for intermittent Windows file-locking failures during
environment creation and installation. The strongest precedent is astral-sh/uv#11002: it contains the
same `RECORD file is invalid` failure caused by another process using the file, remained reproducible
after an earlier concurrency fix, and was explicitly closed by a maintainer as a duplicate of
astral-sh/uv#2810. The antivirus environment is also covered by the broader tracking resource
astral-sh/uv#20792, but the current report does not establish which process owns the lock.

## Current investigation status

A maintainer referred the reporter to astral-sh/uv#20792 and asked for the AV/EDR vendor name and
version, plus whether the reporter has contacted the vendor through its commercial representative.
The reporter identified Trellix as the vendor, but has not supplied a product or version. They plan to
ask their internal information-systems department about filing a vendor bug report; no vendor ticket
has been reported yet. This makes AV/EDR interference the current investigation path, but does not
establish Trellix as the process holding the lock.

The reporter is considering a wrapper that retries uv as a local workaround and asked whether uv
could retry this failure internally. No wrapper implementation or validation was provided. Open
astral-sh/uv#21489 explores additional retries for Windows AV/EDR cache failures, but it handles
`PermissionDenied`/os error 5 rather than the reported os error 33, so it is not evidence that this
specific path is covered.

## Draft response

Thanks for the report. This matches the intermittent Windows file-locking failures tracked in
astral-sh/uv#2810. In particular, the same `RECORD file is invalid` installation failure was reported
in astral-sh/uv#11002, which was consolidated into that issue after it remained reproducible following
astral-sh/uv#11007.

The antivirus/EDR context may be relevant and is tracked in astral-sh/uv#20792, but this error alone
does not identify which process holds the lock. Please add any reliable reproduction details to
astral-sh/uv#2810, especially whether the Jenkins jobs run concurrently or share a uv cache, and
whether the failure still occurs with the current uv release. If your AV/EDR vendor is implicated,
please also share the vendor and its support ticket ID as requested in astral-sh/uv#20792.

## Classification

This is a duplicate because the same intermittent Windows installation problem is already centralized
in open astral-sh/uv#2810. That issue covers `uv venv` and installation failures on Jenkins, retryable
Windows lock errors, shared-cache concurrency, and the identical os error 33 wording. More decisively,
astral-sh/uv#11002 records the exact `RECORD file is invalid` symptom and was explicitly deduplicated
into astral-sh/uv#2810 by a maintainer in May 2026.

Merged astral-sh/uv#11007 added locking around wheel extraction and other cache writes in an attempt to
close astral-sh/uv#11002. The exact RECORD failure was subsequently reproduced with uv 0.6.13 and
0.7.11, and the report was later consolidated into the still-open astral-sh/uv#2810. The present report
therefore belongs to an active canonical issue rather than representing a newly untracked regression.
Antivirus/EDR interference is plausible in light of astral-sh/uv#20792 and the reporter's environment,
but it is not a confirmed cause here.

## Related

- astral-sh/uv#2810 — Open canonical tracker for intermittent Windows failures during environment
  creation and installation. It includes Jenkins/shared-cache conditions, retry-success behavior, the
  identical os error 33 text, and discussion of antivirus locking.
- astral-sh/uv#11002 — Closed duplicate containing the exact Windows `RECORD file is invalid` failure.
  A maintainer explicitly consolidated it into astral-sh/uv#2810 after the problem remained
  reproducible.
- astral-sh/uv#11007 — Merged historical fix adding locks around wheel extraction and cache writes for
  astral-sh/uv#11002. Later reproductions show that it did not resolve the broader canonical problem.
- astral-sh/uv#20792 — Open Windows antivirus/EDR tracking resource covering similar file-access
  failures and requesting the vendor and support ticket ID. It supports the relevance of the reported
  environment but does not prove that antivirus owns this lock.
- astral-sh/uv#21489 — Open pull request proposing request-level retries for AV/EDR-related Windows
  cache failures and citing astral-sh/uv#2810. It targets `PermissionDenied`/os error 5, while this
  report fails with os error 33 while reading `RECORD`, so its coverage is unconfirmed.

## Search evidence

Literal searches covered `RECORD file is invalid`, `locked a portion of the file`, `os error 33`,
Windows, virtual-environment creation, and wheel installation. Conceptual searches covered intermittent
failures that succeed after retries, concurrent or shared cache access, antivirus/EDR locks, access
denied and sharing violations, archive/cache persistence, and atomic file replacement. Fix-oriented
inspection covered merged astral-sh/uv#11007, open astral-sh/uv#21489, and the issues and comments linked
from them.

Two superficially strong matches were ruled out as canonical. astral-sh/uv#14345 reports the same
top-level RECORD message, but its deterministic Linux failure is malformed CSV caused by unescaped
commas in filenames. astral-sh/uv#2767 and its merged fix astral-sh/uv#2800 contain the identical os
error 33 wording, but concern acquisition of the virtual environment's `.lock` file rather than
reading or installing wheel RECORD/cache contents.
