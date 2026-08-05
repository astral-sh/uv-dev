# Feature Request: Add MSYS2 environment detection via `$MINGW_PREFIX`

Issue: astral-sh/uv#20953
Classification: duplicate

## Summary

astral-sh/uv#3573 is the closest open canonical issue; astral-sh/uv#1251 is its broad historical precursor, and astral-sh/uv#3632 is a closed, unmerged attempt to reproduce the failure in CI.

## Classification

astral-sh/uv#3573 already tracks the same underlying requested capability: using MSYS2-provided Python successfully, recognizing its mingw platform, and respecting its Unix-style virtual-environment layout. The new issue proposes MINGW_PREFIX as a specific implementation approach and adds optional guidance and policy ideas, but those additions do not displace the existing canonical compatibility discussion. No evidence indicates a previously fixed regression.

## Related

- https://github.com/astral-sh/uv/issues/3573 (open issue): Unknown operation system: mingw_x86_64_ucrt
  This is the canonical open match. It reports uv venv rejecting an MSYS2/UCRT64 Python with the same exact platform error and explicitly covers both platform recognition and the required bin rather than Scripts virtual-environment layout. Maintainer investigation also confirms MSYS2 Python path-handling complications. The MINGW_PREFIX proposal adds a possible detection mechanism and broader warning policy, but targets the same underlying MSYS2 Python support gap tracked by astral-sh/uv#3573.
- https://github.com/astral-sh/uv/issues/1251 (closed issue): Support msys2 python
  Historical precursor requesting MSYS2 Python support and identifying assumptions that Windows always uses Scripts rather than bin. It was closed after a maintainer believed handling already existed, without a linked fix; the later reproducible failure remains tracked by astral-sh/uv#3573.
- https://github.com/astral-sh/uv/pull/3632 (closed pull request): Add mingw64 system test
  This unmerged pull request explicitly attempted to reproduce astral-sh/uv#3573 in CI. It was closed because the failure was not resolved, so it provides direct evidence that the same MSYS2 compatibility problem was investigated but not fixed.
