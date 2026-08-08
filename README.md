# `uv self update` not picking up latest `uv` version

Issue: astral-sh/uv#21007

Classification: bug

## Summary

On Windows, uv 0.12.2 reports that it is already the latest version instead of updating to the
published 0.12.3 release. The supplied trace shows a successful request to the official mirrored
version manifest at `https://releases.astral.sh/github/versions/main/v1/uv.ndjson`, followed by
`Resolved self-update target to uv==0.12.2`. This is not a certificate or connection failure.

The release and metadata state confirms the incorrect behavior:

- uv 0.12.3 is a non-draft, non-prerelease GitHub release published at 2026-08-07 16:34:42 UTC and
  includes an `x86_64-pc-windows-msvc` artifact in the canonical version manifest.
- At 2026-08-08 11:11 UTC, the official Astral mirror returned a valid manifest whose first and
  newest entry was 0.12.2. Its `Last-Modified` header was 2026-08-05 19:24:11 UTC, before the 0.12.3
  release, and it contained no 0.12.3 entry.
- At the same time, the canonical manifest at
  `https://raw.githubusercontent.com/astral-sh/versions/main/v1/uv.ndjson` started with 0.12.3.

The current implementation prefers the Astral mirror and falls back to the canonical manifest when
fetching or parsing fails. Because the stale mirrored manifest is valid and contains a matching
Windows artifact for 0.12.2, uv accepts it and never reaches the current canonical manifest.

## Draft response

Thanks for the trace. It confirms that uv successfully fetches the official mirrored version
manifest, but that manifest currently ends at 0.12.2 even though 0.12.3 is published; the canonical
manifest already contains 0.12.3. This is therefore different from astral-sh/uv#18701, and
`--system-certs` would not affect it.

The next step is to restore or refresh the mirrored manifest. Until then, you can install 0.12.3
with the version-specific Windows installer from the 0.12.3 release.

## Classification

This is a bug. The command successfully obtains official metadata but incorrectly says 0.12.2 is
the latest version after 0.12.3 was published. Repository code and the two live manifests establish
the stale preferred metadata source; no network or certificate hypothesis is needed.

This is not a duplicate. Searches found no open issue or pull request already tracking the stale
0.12.3 mirror incident. The closest historical issue, astral-sh/uv#18701, failed before any manifest
could be read because of an untrusted certificate and was resolved for the reporter by
`--system-certs`; astral-sh/uv#21007 successfully reads a different, stale response.

## Related

- astral-sh/uv#18679 — “Make `uv self update` fetch the manifest from the mirror first” (merged).
  This is the implementation most directly connected to the failure. It made the official updater
  prefer the Astral mirror and use the canonical raw manifest as fallback. A successful but stale
  mirror response resolves 0.12.2 and does not trigger fallback.
- astral-sh/uv#18503 — “Fetch release assets from `releases.astral.sh` first” (closed). This tracking
  issue records the completed move of uv self-update release metadata to the Astral mirror. The new
  report is a failure of that completed path, rather than the same request.
- astral-sh/uv#18701 — “`uv self update` failing to resolve latest uv version” (closed). It affects
  the same command and resolution stage, but the trace showed `invalid peer certificate:
  UnknownIssuer` behind a corporate proxy and `--system-certs` worked. The current issue's request
  succeeds, so this is an adjacent but distinct failure.

## Search coverage

Open and closed issues and open, closed, and merged pull requests were searched separately using
literal combinations of `self update`, `latest version`, `newest`, `0.12.2`, `0.12.3`,
`system-certs`, the exact manifest URL, and `Resolved self-update target`. Conceptual searches used
`stale release`, `delayed release`, `version manifest`, `release manifest`, mirror, updater,
Windows, and GitHub rate-limit/proxy terminology. Fix-oriented searches covered closed issues and
merged pull requests for manifest resolution, the Astral mirror migration, and version-specific
self-update failures.

The strongest implementation chain was followed from astral-sh/uv#18503 through
astral-sh/uv#18674 and astral-sh/uv#18679. astral-sh/uv#18674 introduced canonical-manifest version
resolution, while astral-sh/uv#18679 put the Astral mirror ahead of that current canonical source.
The latter is the materially closer result.

Plausible issue candidates were inspected and ruled out as the same bug. The open
astral-sh/uv#5514 reports a macOS command hanging indefinitely at `Checking for updates...`, not a
successful lookup that selects an old release. astral-sh/uv#16309 reached the latest-version lookup
but failed later while the Windows installer downloaded the update; astral-sh/uv#18592 also found
and installed the requested update before antivirus removed it; astral-sh/uv#15073 concerns a
misleading error only when GitHub rate limiting affects an explicitly requested version. None
reports a valid official manifest lagging behind a published release.

## Supporting evidence

- The reporter's trace successfully receives the mirror response, then logs
  `Resolved self-update target to uv==0.12.2` and the incorrect latest-version message.
- The 0.12.3 release was published roughly 18.5 hours before astral-sh/uv#21007 was opened, so this
  is not a report filed during the release publication itself.
- The mirrored manifest remained at 0.12.2 and retained an August 5 modification time when checked
  during triage; the canonical manifest already contained 0.12.3 for the reported Windows target.
- The source in `crates/uv-bin-install/src/lib.rs` orders the mirror before the canonical manifest,
  and `crates/uv/src/commands/self_update.rs` labels the first resolved manifest entry as the latest
  version when no explicit target was requested.
