# `uv self update` not picking up latest `uv` version

Issue: astral-sh/uv#21007

Classification: bug

## Summary

On Windows, uv 0.12.2 reports that it is already the latest version instead of updating to the
published 0.12.3 release. A targeted reproduction with the official uv 0.12.2 Linux binary produced
the same result: the updater successfully requested the official mirrored version manifest at
`https://releases.astral.sh/github/versions/main/v1/uv.ndjson`, resolved `uv==0.12.2`, and left the
binary on 0.12.2. This is not a certificate or connection failure.

The release and metadata state confirms the incorrect behavior:

- uv 0.12.3 is a non-draft, non-prerelease GitHub release published at 2026-08-07 16:34:42 UTC and
  includes an `x86_64-pc-windows-msvc` artifact in the canonical version manifest.
- At 2026-08-08 11:15 UTC, the official Astral mirror returned a valid manifest whose first and
  newest entry was 0.12.2. Its `Last-Modified` header was 2026-08-05 19:24:11 UTC, before the 0.12.3
  release, and it contained no 0.12.3 entry.
- At the same time, the canonical manifest at
  `https://raw.githubusercontent.com/astral-sh/versions/main/v1/uv.ndjson` started with 0.12.3.

The current implementation prefers the Astral mirror and falls back to the canonical manifest when
fetching or parsing fails. Because the stale mirrored manifest is valid and contains a matching
Windows artifact for 0.12.2, uv accepts it and never reaches the current canonical manifest.

## Reproduction

Outcome: **reproducible**.

The reproduction used an isolated temporary directory, the official uv 0.12.2
`x86_64-unknown-linux-gnu` release archive, and a temporary standalone-install receipt pointing at
that binary. No checkout files or existing installation state were changed. With credentials
removed from the subprocess environment, the meaningful commands were:

```console
$ uv --version
uv 0.12.2 (x86_64-unknown-linux-gnu)

$ uv self update --no-cache --no-progress -v
info: Checking for updates...
DEBUG Using official public self-update path
DEBUG Resolved self-update target to `uv==0.12.2`
success: You're already on version v0.12.2 of uv (the latest version).

$ uv --version
uv 0.12.2 (x86_64-unknown-linux-gnu)
```

Adding `--system-certs` produced the same resolution and latest-version message. A `--dry-run`
lookup also produced the same result.

At the time of reproduction, inspecting the first entry of each official manifest showed:

```text
https://releases.astral.sh/github/versions/main/v1/uv.ndjson
version = 0.12.2; date = 2026-08-05T19:22:56Z; x86_64-pc-windows-msvc artifact = present

https://raw.githubusercontent.com/astral-sh/versions/main/v1/uv.ndjson
version = 0.12.3; date = 2026-08-07T16:34:42Z; x86_64-pc-windows-msvc artifact = present
```

The executable reproduction ran on Linux rather than the reporter's Windows
10.0.26200.8973/x86_64 environment. That does not account for the discrepancy: the same stale
mirror entry contains an artifact for the reported `x86_64-pc-windows-msvc` target, and the
reporter's trace independently shows the same successful request and `uv==0.12.2` resolution.
Python 3.14.7 is not involved in self-update version selection.

There is no existing test for a valid but stale preferred manifest returning an older latest
version than the canonical fallback. Relevant coverage is:

- `crates/uv/tests/it/self_update.rs::check_self_update` exercises an end-to-end self-update but
  only asserts that the resulting binary runs; it does not compare preferred and canonical
  manifests or assert the selected version.
- `crates/uv-bin-install/src/lib.rs::test_manifest_falls_back_on_404` and
  `test_manifest_falls_back_on_parse_error` verify fallback for failed or invalid mirror responses.
- `crates/uv-bin-install/src/lib.rs::test_manifest_no_matching_version_does_not_fallback` verifies
  that a valid mirror response with no constrained match does not query the canonical URL. None of
  these tests covers a successful stale response for an unconstrained latest-version lookup.

## Draft response

Thanks for the trace. It confirms that uv successfully fetches the official mirrored version
manifest, but that manifest currently ends at 0.12.2 even though 0.12.3 is published; the canonical
manifest already contains 0.12.3. This is therefore different from astral-sh/uv#18701, and
`--system-certs` would not affect it.

The next step is to restore or refresh the mirrored manifest. Until then, you can install 0.12.3
with the version-specific Windows installer from the 0.12.3 release.

## Classification

This is a bug. The targeted command successfully obtains official metadata but incorrectly says
0.12.2 is the latest version after 0.12.3 was published. The command output, the two live manifests,
and the fallback implementation confirm that the successful stale preferred response produces the
observed selection; no network or certificate hypothesis is needed.

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

- Both the reporter's trace and the isolated reproduction successfully receive the mirror response,
  then log `Resolved self-update target to uv==0.12.2` and the incorrect latest-version message.
- The 0.12.3 release was published roughly 18.5 hours before astral-sh/uv#21007 was opened, so this
  is not a report filed during the release publication itself.
- The mirrored manifest remained at 0.12.2 and retained an August 5 modification time when checked
  during triage; the canonical manifest already contained 0.12.3 for the reported Windows target.
- The source in `crates/uv-bin-install/src/lib.rs` orders the mirror before the canonical manifest,
  and `crates/uv/src/commands/self_update.rs` labels the first resolved manifest entry as the latest
  version when no explicit target was requested.
