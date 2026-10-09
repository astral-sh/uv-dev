# uv self update does not use astral mirror value when logging

Issue: astral-sh/uv#22264

Classification: bug

## Summary

The reporter observes that `uv self update` honors `UV_ASTRAL_MIRROR_URL` for downloads, including passing `UV_DOWNLOAD_URL` to the installer, but prints a GitHub release-page link on success. The example upgrades from 0.12.19 to 0.12.23 on Linux or Windows 11:

```text
success: Upgraded uv from v0.12.19 to v0.12.23! https://github.com/astral-sh/uv/releases/tag/0.12.23
```

The requested behavior is for the displayed URL to reflect the configured mirror. The misleading link confuses users in an organization that requires downloads through proxy repositories; the reporter explicitly describes this as an output problem, not a failed or misrouted download.

The logging mismatch was reproduced with the installed uv 0.12.13 executable, a local HTTP mirror, and a mock installer that exits successfully without replacing binaries. Metadata and installer requests reached the mirror, and the installer received the mirrored artifact directory through `UV_DOWNLOAD_URL`, while the success message linked to GitHub. This establishes the output behavior, not an actual upgrade to 0.12.23 or a download-routing failure.

Prior issue research identified an open fix in astral-sh/uv#22373 and an integration reproduction in astral-sh/uv-dev#2302. Earlier mirror support is established by astral-sh/uv#18525 and astral-sh/uv#19206; astral-sh/uv#22267 concerns separate standalone installer behavior.

## Classification

Bug: misleading success-message URL when a custom Astral mirror is configured. The mismatch is confirmed by executable behavior and is consistent with the checkout's official updater formatting a GitHub release-page URL independently of mirror selection. A release-page link is informational; its presence does not establish that downloads bypassed the mirror. No previously fixed logging behavior was found to establish a regression. The exact-match implementation and test PRs were opened in response to this issue, so they do not justify duplicate classification.

A maintainer explicitly welcomed a small PR fixing the log on 2026-10-07, according to prior issue research. Mirror support was released in 0.11.14, before the reported 0.12.19–0.12.23 range. The historical mirror changes concerned metadata and artifact routing, not a previously corrected success-message URL. The report provides no last known-good version and does not establish an upgrade regression.

## Reproduction

**Outcome: reproducible.** A targeted command execution on Linux produced the reported mismatch. This used a mock successful installer to exercise logging without updating any installed binary.

### Reported and tested environments

- Report: `uv self update`, `UV_ASTRAL_MIRROR_URL` configured; uv 0.12.19–0.12.23; Linux 6.8.0-138-generic x86_64 and Windows 11 x64. The mirror's literal URL and receipt were not supplied. Python version was not supplied and is not relevant to this command.
- Tested: uv 0.12.13 (`x86_64-unknown-linux-gnu`), provided on PATH at `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`; Linux 6.17.0-1022-azure x86_64. Python 3.12.3 served only as the local fixture harness.
- A byte-for-byte copy of that installed executable ran from `/tmp/uv-22264-epx6O5/bin/uv`. No checkout binary was built. The exact reported starting version and Windows were not executed.

### Fixture and command

The runnable fixture is `/tmp/uv-22264-epx6O5/reproduce.py`; its captured results are `/tmp/uv-22264-epx6O5/evidence.json`. Run it with:

```sh
python3 /tmp/uv-22264-epx6O5/reproduce.py
```

The harness starts an HTTP server on an ephemeral localhost port. In the observed run, `UV_ASTRAL_MIRROR_URL` was `http://127.0.0.1:42387/mirror`. It serves exactly these successful responses:

1. `/mirror/github/versions/main/v1/uv.ndjson`: one manifest entry with version `0.12.23`, date `2026-10-03T00:00:00Z`, and an `x86_64-unknown-linux-gnu` artifact with `archive_format: "tar.gz"` and the canonical URL `https://github.com/astral-sh/uv/releases/download/0.12.23/uv-x86_64-unknown-linux-gnu.tar.gz`.
2. `/mirror/github/uv/releases/download/0.12.23/uv-installer.sh`: a shell script that writes only its `UV_DOWNLOAD_URL` value to a designated temporary file and exits 0. It does not download an archive or replace an executable.

A synthetic `uv-receipt.json` points `install_prefix` at the temporary `bin` directory, lists `binaries: ["uv"]` and `cdylibs: []`, and sets the source to `release_type: "github"`, `owner: "astral-sh"`, `name: "uv"`, `app_name: "uv"`. Its version is `0.12.13`, its provider is `cargo-dist` version `0.31.0`, and `modify_path` is false. This follows the existing integration-test receipt setup while disabling shell modifications.

The command under test is `uv self update`, with the copied executable first on PATH and these relevant settings:

```sh
UV_ASTRAL_MIRROR_URL=http://127.0.0.1:42387/mirror
AXOUPDATER_CONFIG_PATH=/tmp/uv-22264-epx6O5/receipt
UV_NO_CONFIG=1
UV_NO_MODIFY_PATH=1
```

The subprocess receives a newly constructed environment, with HOME, XDG configuration/data/cache directories, `UV_CACHE_DIR`, and `TMPDIR` all under the temporary fixture directory. It runs there, with no inherited installer URL overrides or credentials. The mock installer records only the fixture-generated download URL.

### Observed result

Exit status was 0, stdout was empty, and stderr was:

```text
info: Checking for updates...
success: Upgraded uv from v0.12.13 to v0.12.23! https://github.com/astral-sh/uv/releases/tag/0.12.23
```

The local server recorded both manifest and installer GET requests listed above. The installer captured:

```text
http://127.0.0.1:42387/mirror/github/uv/releases/download/0.12.23
```

Thus mirror selection and `UV_DOWNLOAD_URL` propagation were observed, but the success message contained the GitHub release-page URL and no mirror URL. The expected behavior in the report is a success link reflecting the configured mirror. The exact replacement link remains a design choice: an artifact mirror need not host GitHub-style release-note pages.

Both the original executable and its temporary copy retained their initial SHA-256 digest. This was a real execution of uv's updater with mocked metadata and installer execution, not a real binary upgrade or an artifact-download test. No external network requests were needed; the local request log is not a general network trace.

### Existing test coverage

Read the setup and assertions in `crates/uv/tests/it/self_update.rs`. `check_self_update` performs an update and checks that the resulting binary accepts `--version`, but does not configure an Astral mirror or assert the success URL. `test_self_update_uses_custom_path_with_ghe_override` and `test_self_update_uses_legacy_path_with_ghe_override` use mocked GitHub Enterprise metadata and `--dry-run`; they do not reach the official updater's success message. The remaining tests cover help, offline handling, dry-run/no-op behavior, and quiet output. No existing integration assertion of this mismatch was found in that module or by searching `crates/uv/tests/` and `crates/uv-client/tests/it/` for the mirror and success-message identifiers.

In `crates/uv/src/commands/self_update.rs`, `test_official_installer_urls_custom_astral_mirror` asserts that a custom mirror produces only the mirrored installer URL; `test_installer_download_url_custom_astral_mirror` checks the artifact directory; and the Unix-only `test_execute_official_installer_sets_download_url_for_astral_mirror` executes a recording shell script and asserts its `UV_DOWNLOAD_URL`. These support the fixture design but do not assert final success logging. The integration module is gated by `self-update` in `crates/uv/tests/it/main.rs`.

No repository tests were added or run. The targeted fixture passed its checks for exit status, both mirror requests, the installer environment, the GitHub success link, and unchanged executable hashes.

## Related

The following statuses and descriptions are retained from prior issue research; this reproduction pass did not re-query GitHub.

- astral-sh/uv#22373 — fix(self-update): show custom mirror URL in success message (open pull request). Explicitly fixes astral-sh/uv#22264. The current revision displays the downloader-returned installer URL for custom mirrors. Opened after this report; its existence does not make the issue a duplicate.
- astral-sh/uv-dev#2302 — Add regression test for astral-sh/uv#22264 (open pull request). Linked from the issue timeline and adds an integration reproduction of this exact report. This is a test-only follow-up, not a released fix or an independent earlier report.
- astral-sh/uv#19206 — Add Astral mirror URL override (merged pull request). Implemented UV_ASTRAL_MIRROR_URL for self-update metadata and artifacts without GitHub fallback, closing astral-sh/uv#18525. Released in 0.11.14; this establishes the existing mirror functionality but did not fix the success-message URL.
- astral-sh/uv#18525 — Add support for customizing Astral mirror URLs (closed issue). Canonical historical request for overriding Astral download URLs in restricted environments, implemented by astral-sh/uv#19206. It concerns download routing, not the remaining misleading success link.
- astral-sh/uv#22267 — standalone installer - UV_ASTRAL_MIRROR_URL is not used when set (open issue). Closely adjacent mirror report, but concerns standalone scripts ignoring the variable during downloads. Here self-update already passes the mirror through UV_DOWNLOAD_URL and only the success message is incorrect.

## Supporting evidence

- In checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`, `crates/uv/src/commands/self_update.rs:346` reads the custom mirror and passes it through installer selection and execution. Line 388 independently formats the GitHub release-page URL in the success message.
- `official_installer_urls_with_mirror` at `crates/uv/src/commands/self_update.rs:405` builds the custom-mirror installer URL. Its GitHub fallback is added only when no custom mirror is configured.
- `installer_download_url` at `crates/uv/src/commands/self_update.rs:44` constructs the mirror artifact directory, preserving the mirror path prefix and trimming trailing slashes. `execute_official_installer` sets `UV_DOWNLOAD_URL` from it at line 544. Local executable reproduction confirmed this environment propagation; actual artifact downloading was not tested.
- `crates/uv-static/src/env_vars.rs` documents the override for Astral metadata and artifact downloads, with no GitHub fallback when configured. It also documents the more-specific installer settings that take precedence.
- `changelogs/0.11.x.md:532` records release 0.11.14 on 2026-05-12 and includes astral-sh/uv#19206. Its discussion explains that overriding the manifest URL enables self-update through corporate mirrors.
- The earlier merged astral-sh/uv#18682 moved official self-update installation to the Astral mirror. Its manual test output already combines a mirror download with a GitHub success link. This is historical context, not a fix for the reported logging problem.
- astral-sh/uv#22373 was opened on 2026-10-08, after this issue. Its body explicitly targets this report. A maintainer requested returning the used URL from the downloader instead of reconstructing it. The inspected head, `2434b8ac4f424628cdc9043bc5870c112ec977ec`, returns that URL and selects it for custom-mirror success output. The PR remains open and unmerged.
- astral-sh/uv-dev#2302 is linked from the issue timeline and supplies an integration reproduction. That PR's test was not executed here; the local executable reproduction above is independent evidence.

## Search coverage and exclusions

Prior issue research searched open and closed astral-sh/uv issues with UV_ASTRAL_MIRROR_URL, UV_DOWNLOAD_URL, Upgraded uv, self update plus mirror/logging/success/GitHub URL/release notes/hardcoded, and area:tracing and area:error-messages labels, without version or platform restrictions. It attempted all-state PR searches for the identifiers, mirror support, logging, and the issue number. REST searches hit a rate limit, and GraphQL PR searches omitted known matching PRs; the research supplemented them with issue timelines, the 100 most recent PRs across states, and direct inspection of linked open and merged PRs, their comments, and source. PR search coverage is therefore incomplete. Historical fixes and release timing were checked through astral-sh/uv#19206 and astral-sh/uv#18682. astral-sh/uv#16519 (update download failure), astral-sh/uv#18412 (quiet/cron output control), and astral-sh/uv#4748 (displaying the previous version) were ruled out as duplicates.

The symptom searches were kept separate from download-routing and historical implementation searches. The strongest candidates were compared using their bodies, comments, timelines, creation dates, and source where available. No independent earlier report of the same logging mismatch was established.

astral-sh/uv#22267 was inspected and retained as an adjacent issue because the shared variable and deployment setting could otherwise suggest a duplicate. Its missing standalone-installer support changes download selection; this report confirms that self-update downloads already use the mirror. astral-sh/uv#18503 is broader completed mirror-adoption history and does not track the success-message correction.

## Maintainer next step

Review and coordinate the existing fix in astral-sh/uv#22373, using the reproduction in astral-sh/uv-dev#2302 where useful. The current GitHub link is a release page, while the mirror serves metadata and artifacts; a mirrored `releases/tag` page should not be assumed to exist. The proposed fix uses the successful installer URL for custom mirrors.

Any additional test should follow the existing integration snapshot style and applicable feature/platform gates. See the reproduction section for the inspected coverage and the limits of the observed result.

Only this handoff README and temporary reproduction files were written. No checkout files, installed binaries, existing user configuration, or GitHub state were changed. No builds or repository test suites were run.
