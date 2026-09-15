# 0.12.14: `uv pip install --system` fails in official `python:*` Docker images — "Cannot install into symlinked directory: /usr/local/man"

Issue: astral-sh/uv#21692

Classification: bug

## Summary

uv 0.12.14 rejects installation of a wheel whose `.data/data` contents map to an existing symlinked directory in the target installation scheme. The supplied reproduction installs `clevercsv==0.8.4` with `uv pip install --system` in Debian-based official Python images, where `/usr/local/man` is a relative symlink to `share/man`; uv fails with `The wheel is invalid: Cannot install into symlinked directory: /usr/local/man`. The reporter's version comparison says the same install succeeds through uv 0.12.13.

Repository evidence confirms the new failure path. astral-sh/uv#21569 added `ValidatedWheelDestination` and the exact error, including validation of a wheel's `.data/data` subtree against the installation scheme's data root. It merged after the 0.12.13 release and is present in 0.12.14. The validation rejects any existing destination directory symlink encountered for a wheel directory, without resolving the link and checking whether it remains inside the trusted installation prefix. The 0.12.14 release notes do not list astral-sh/uv#21569.

No existing issue or pull request was found that already tracks this specific 0.12.14 regression or fixes it.

## Draft response

Thanks for the detailed reproduction. astral-sh/uv#21569 introduced the destination-symlink check in uv 0.12.14 to prevent a wheel payload from being written outside its installation environment. The current check also rejects `/usr/local/man` without considering that its relative target, `/usr/local/share/man`, remains within the same installation prefix, so this is a regression rather than an invalid wheel.

The next step is to add coverage for an existing scheme-directory symlink whose resolved target stays within the prefix and adjust the validation without weakening the outside-prefix protection. In the meantime, replacing `/usr/local/man` with a directory or pinning uv 0.12.13 are the available workarounds from the reproduction.

## Classification

This is a **bug**, not a duplicate. The current source establishes that uv 0.12.14 unconditionally rejects a pre-existing directory symlink encountered beneath a wheel destination. In the reported standard system layout, that turns a valid wheel installation into a failure even though `/usr/local/man` resolves within `/usr/local`. The user-facing `The wheel is invalid` cause is also misleading because the rejection is determined by the destination layout, not malformed wheel contents.

astral-sh/uv#21569 is the causative historical change, but it does not track this regression and therefore is not a canonical duplicate. No open issue or pull request already tracks the same regression.

## Related

- astral-sh/uv#21569 — **merged pull request, “Reject symlinked wheel installation destinations.”** This is the closest result and the direct source of the behavior. It added the exact error and validates `.data/data` destinations to prevent installation through directory symlinks that could redirect writes outside the environment. Its implementation does not distinguish `/usr/local/man -> share/man`, whose resolved target remains under the installation prefix. It merged on 2026-09-10 after uv 0.12.13 was released and shipped in uv 0.12.14 on 2026-09-15.
- astral-sh/uv#21255 — **open issue, “uv sync and uv pip install fails for packages with console scripts when the venv contains a `lib` to `usr/lib` symlink.”** This is adjacent symlink-related installer work, but not the same problem. It predates uv 0.12.14, uses a synthetic venv merged-`/usr` layout, and concerns relative console-script path calculation that ends in a metadata lookup failure. Its open proposed fix, astral-sh/uv#21256, normalizes or resolves script paths; it does not address `.data/data` destination validation.

## Search and supporting evidence

Searches covered open and closed issues and open, closed-unmerged, and merged pull requests. Literal searches used the exact error, `/usr/local/man`, `share/man`, `.data/data`, and `uv pip install --system`. Conceptual searches covered wheel data files, mapped installation destinations, symlinked scheme roots, links that remain within a prefix versus links that escape an environment, official Python Docker layouts, and man pages. Fix-oriented searches checked merged work behind 0.12.14 and newer open or merged pull requests.

The strongest ruled-out candidates were:

- astral-sh/uv#15243, which concerns `uv_build` traversing directory symlinks in a package source tree, a different subsystem and direction of operation.
- astral-sh/uv#4731 and astral-sh/uv#11354, which request ways to expose man pages and shell completions installed inside `uv tool` environments; they do not concern system-scheme wheel data installation.
- astral-sh/uv#18942, which protects uninstall operations from malicious `RECORD` entries outside an environment. It is part of the broader environment-boundary work referenced by astral-sh/uv#21569, but it does not produce or track this install-time regression.

Current tests added by astral-sh/uv#21569 cover rejecting symlinked package, nested package, data-package, and headers destinations when those links point to external directories. They do not cover a standard installation-scheme directory symlink whose resolved target remains within the same trusted prefix.
