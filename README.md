# uv self update does not use astral mirror value when logging

Issue: astral-sh/uv#22264

Classification: bug

## Summary

The reporter observes that `uv self update` honors `UV_ASTRAL_MIRROR_URL` for downloads, including passing `UV_DOWNLOAD_URL` to the installer, but prints a GitHub release-page link on success. The example upgrades from 0.12.19 to 0.12.23 on Linux or Windows 11:

```text
success: Upgraded uv from v0.12.19 to v0.12.23! https://github.com/astral-sh/uv/releases/tag/0.12.23
```

The requested behavior is for the displayed URL to reflect the configured mirror. The misleading link confuses users in an organization that requires downloads through proxy repositories; the reporter explicitly describes this as an output problem, not a failed or misrouted download.

A logging bug with an open fix in astral-sh/uv#22373 and a reproduction in astral-sh/uv-dev#2302. Earlier mirror support is established by astral-sh/uv#18525 and astral-sh/uv#19206; astral-sh/uv#22267 concerns separate standalone installer behavior.

## Draft response

You're right: the official updater passes the configured mirror to the installer, but its success message independently hard-codes a GitHub release link. That printed link does not establish that the download used GitHub. A fix is open in astral-sh/uv#22373 to display the actual installer URL when a custom mirror is configured. Please coordinate any further contribution there; it has not merged yet.

## Classification

The official updater hard-codes a GitHub release-page URL in its success message despite using the configured mirror for installation. This is source-confirmed misleading output. No previously fixed logging behavior was found to establish a regression. The exact-match implementation and test PRs were opened in response to this issue, so they do not justify duplicate classification.

A maintainer explicitly welcomed a small PR fixing the log on 2026-10-07. The source evidence establishes the bug without requiring an additional reproduction from the reporter. Mirror support was released in 0.11.14, before the reported 0.12.19–0.12.23 range. The historical mirror changes concerned metadata and artifact routing, not a previously corrected success-message URL.

## Related

- astral-sh/uv#22373 — fix(self-update): show custom mirror URL in success message (open pull request). Explicitly fixes astral-sh/uv#22264. The current revision displays the downloader-returned installer URL for custom mirrors. Opened after this report; its existence does not make the issue a duplicate.
- astral-sh/uv-dev#2302 — Add regression test for astral-sh/uv#22264 (open pull request). Linked from the issue timeline and adds an integration reproduction of this exact report. This is a test-only follow-up, not a released fix or an independent earlier report.
- astral-sh/uv#19206 — Add Astral mirror URL override (merged pull request). Implemented UV_ASTRAL_MIRROR_URL for self-update metadata and artifacts without GitHub fallback, closing astral-sh/uv#18525. Released in 0.11.14; this establishes the existing mirror functionality but did not fix the success-message URL.
- astral-sh/uv#18525 — Add support for customizing Astral mirror URLs (closed issue). Canonical historical request for overriding Astral download URLs in restricted environments, implemented by astral-sh/uv#19206. It concerns download routing, not the remaining misleading success link.
- astral-sh/uv#22267 — standalone installer - UV_ASTRAL_MIRROR_URL is not used when set (open issue). Closely adjacent mirror report, but concerns standalone scripts ignoring the variable during downloads. Here self-update already passes the mirror through UV_DOWNLOAD_URL and only the success message is incorrect.

## Supporting evidence

- In checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`, `crates/uv/src/commands/self_update.rs:346` reads the custom mirror and passes it through installer selection and execution. Line 388 independently formats the GitHub release-page URL in the success message.
- `official_installer_urls_with_mirror` at `crates/uv/src/commands/self_update.rs:405` builds the custom-mirror installer URL. Its GitHub fallback is added only when no custom mirror is configured.
- `installer_download_url` at `crates/uv/src/commands/self_update.rs:44` constructs the mirror artifact directory, preserving the mirror path prefix and trimming trailing slashes. `execute_official_installer` sets `UV_DOWNLOAD_URL` from it at line 544. These are source-backed routing facts; no network trace or live update was performed.
- `crates/uv-static/src/env_vars.rs:562` documents the override for Astral metadata and artifact downloads, with no GitHub fallback when configured. It also documents the more-specific installer settings that take precedence.
- `changelogs/0.11.x.md:532` records release 0.11.14 on 2026-05-12 and includes astral-sh/uv#19206. Its discussion explains that overriding the manifest URL enables self-update through corporate mirrors.
- The earlier merged astral-sh/uv#18682 moved official self-update installation to the Astral mirror. Its manual test output already combines a mirror download with a GitHub success link. This is historical context, not a fix for the reported logging problem.
- astral-sh/uv#22373 was opened on 2026-10-08, after this issue. Its body explicitly targets this report. A maintainer requested returning the used URL from the downloader instead of reconstructing it. The inspected head, `2434b8ac4f424628cdc9043bc5870c112ec977ec`, returns that URL and selects it for custom-mirror success output. The PR remains open and unmerged.
- astral-sh/uv-dev#2302 is linked from the issue timeline and supplies an integration reproduction. Its existence is useful for review, but no test execution or verification of its result was performed here.

## Search coverage and exclusions

Used authenticated gh to search open and closed astral-sh/uv issues with UV_ASTRAL_MIRROR_URL, UV_DOWNLOAD_URL, Upgraded uv, self update plus mirror/logging/success/GitHub URL/release notes/hardcoded, and area:tracing and area:error-messages labels, without version or platform restrictions. Attempted all-state PR searches for the identifiers, mirror support, logging, and the issue number. REST searches hit a rate limit, and GraphQL PR searches omitted known matching PRs; supplemented them with issue timelines, the 100 most recent PRs across states, and direct inspection of linked open and merged PRs, their comments, and source. PR search coverage is therefore incomplete. Checked historical fixes and release timing through astral-sh/uv#19206 and astral-sh/uv#18682. Ruled out astral-sh/uv#16519 (update download failure), astral-sh/uv#18412 (quiet/cron output control), and astral-sh/uv#4748 (displaying the previous version) as duplicates.

The symptom searches were kept separate from download-routing and historical implementation searches. The strongest candidates were compared using their bodies, comments, timelines, creation dates, and source where available. No independent earlier report of the same logging mismatch was established.

astral-sh/uv#22267 was inspected and retained as an adjacent issue because the shared variable and deployment setting could otherwise suggest a duplicate. Its missing standalone-installer support changes download selection; this report confirms that self-update downloads already use the mirror. astral-sh/uv#18503 is broader completed mirror-adoption history and does not track the success-message correction.

## Maintainer next step

Review and coordinate the existing fix in astral-sh/uv#22373, using the reproduction in astral-sh/uv-dev#2302 where useful. The current GitHub link is a release page, while the mirror serves metadata and artifacts; a mirrored `releases/tag` page should not be assumed to exist. The proposed fix uses the successful installer URL for custom mirrors.

Existing tests in `crates/uv/src/commands/self_update.rs` cover custom-mirror installer selection, artifact-directory construction, empty override handling, and propagation of `UV_DOWNLOAD_URL`. Those tests do not directly assert this success-message mismatch. The integration module `crates/uv/tests/it/self_update.rs` is gated by `self-update` in `crates/uv/tests/it/main.rs`; any additional test should follow the existing snapshot style and applicable feature/platform gates.

Only this handoff README was updated. No checkout files or GitHub state were changed, and no builds or tests were run.
