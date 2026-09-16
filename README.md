# frozen block UV_DEFAULT_INDEX and UV_INDEX_STRATEGY

Issue: astral-sh/uv#21747

Classification: duplicate

## Summary

The reporter copies an existing `uv.lock` into a Python 3.14 Docker build, sets a custom package
mirror through `UV_DEFAULT_INDEX` and `UV_INDEX_STRATEGY=unsafe-any-match`, and runs
`uv sync --no-cache -v --frozen` with uv 0.12.15. The frozen sync downloads from the locations
recorded in the lockfile rather than the custom default index. Removing `--frozen` makes the custom
index take effect.

This is the current intended behavior: frozen mode installs directly from the lockfile without
performing resolution, and the lockfile includes concrete registry and artifact URLs. Consequently,
`UV_INDEX_STRATEGY` has no candidate selection to govern, and `UV_DEFAULT_INDEX` does not rewrite
the artifact locations already recorded in the lockfile. The exact symptom was previously confirmed
in astral-sh/uv#19625. The broader request for a portable lockfile that can be installed through a
different proxy or index is tracked in astral-sh/uv#6349, with an open implementation in
astral-sh/uv#20790.

## Draft response

Thanks for the report. With `--frozen`, uv installs from `uv.lock` without performing resolution,
and the lockfile contains the selected registry and artifact URLs. As a result,
`UV_INDEX_STRATEGY` is not consulted, and changing `UV_DEFAULT_INDEX` does not replace URLs already
recorded in the lockfile. This is the same behavior confirmed in astral-sh/uv#19625.

The request to install one lockfile through a different index or proxy is already tracked in
astral-sh/uv#6349, and astral-sh/uv#20790 is the current open implementation for proxy-index routing,
including frozen installs. I would close this as a duplicate of astral-sh/uv#6349. Until that work is
available, the lockfile needs to be generated with the index used for the frozen install, or the
sync must be allowed to update the lockfile when changing indexes.

## Classification

This is a duplicate of astral-sh/uv#6349. Although the report describes the current behavior as a
bug, repository evidence establishes that frozen installs deliberately use the URLs in `uv.lock`:
a maintainer closed the same frozen-sync report in astral-sh/uv#19625 with that explanation. The
requested change is the same portable-index/proxy capability being centralized in the still-open
astral-sh/uv#6349. The open astral-sh/uv#20790 predates this report and implements explicit proxy
routing while preserving canonical lockfile URLs, so this is not a regression of a previously
released fix.

The `UV_INDEX_STRATEGY` portion does not establish a separate defect. Index strategy controls how
the resolver selects candidates across indexes; `--frozen` skips resolution. That differs from
astral-sh/uv#17068, where index strategy is ignored during an actual resolution involving required
environments.

## Related

- astral-sh/uv#6349 — Open enhancement and canonical discussion for using the same lockfile across
  machines and CI environments with different index or proxy URLs. Its current design discussion
  explicitly covers routing canonical locked artifacts through a configured proxy.
- astral-sh/uv#19625 — Closed issue reporting the same observable behavior: `uv sync --frozen`
  ignores runtime mirror settings, including `UV_DEFAULT_INDEX`, and downloads the concrete URLs in
  `uv.lock`. A maintainer confirmed this is intentional.
- astral-sh/uv#20790 — Open pull request implementing proxy indexes with reversible artifact URLs.
  It preserves canonical URLs in lockfiles, routes existing locked artifacts through a proxy, and
  includes frozen-install coverage. It is not yet merged.

## Supporting evidence

- The project documentation defines `--frozen` as using the lockfile without checking or updating
  it, while the sync command documentation states that a project is not re-locked when `--frozen`
  is provided.
- astral-sh/uv#19625 reproduces the behavior with uv 0.11.14 and 0.11.17 and was closed on
  2026-06-05 as intentional. The current report uses uv 0.12.15; no intervening merged fix was found,
  so there is no evidence of a regression.
- astral-sh/uv#20790 describes explicit proxy configuration rather than changing the meaning of
  `UV_DEFAULT_INDEX`. Its proposed behavior converts canonical locked artifact URLs to proxy URLs
  during installation and specifically calls out frozen and offline installs.

## Search coverage

Literal searches covered `UV_DEFAULT_INDEX`, `UV_INDEX_STRATEGY`, `unsafe-any-match`, `--frozen`,
and custom-index terminology across open and closed issues and open, closed, and merged pull
requests. Conceptual searches covered mirror and proxy routing, lockfile URLs, index portability,
locked artifact sources, and different developer/CI indexes. Fix-oriented searches covered proxy
indexes and version-specific closed reports and merged changes. The strongest discussion chain was
astral-sh/uv#19625 to astral-sh/uv#6349 to astral-sh/uv#20790.

astral-sh/uv#16996 was inspected but concerns warning when frozen mode silently ignores resolution
options, not replacing locked download sources. astral-sh/uv#17068 concerns index-strategy behavior
during required-environment resolution, and astral-sh/uv#20612 was traced to universal resolution
including platform-specific dependencies. None is as close as the canonical chain above.
