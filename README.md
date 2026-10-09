# Python 3.14.8 missing from `uv python list` on x86_64 macOS with `python-downloads = "manual"`

Issue: astral-sh/uv#22184

Classification: duplicate

## Summary

The reporter uses uv 0.12.23 on x86_64 macOS 14.8.9 with Python 3.14.7 installed. Plain `uv python list` omits Python 3.14.8, and `uv python install 3` or `uv python install 3.14` reports that Python is already installed. Explicitly installing 3.14.8 succeeds.

In a follow-up, the reporter identified `python-downloads = "manual"` and confirmed that both `--only-downloads` and `--allow-python-downloads` reveal the version. They also confirmed that `uv python install 3.14 --upgrade` performs the requested patch upgrade. Their remaining request is to reconsider or clarify the listing policy.

astral-sh/uv#13167 is the exact listing-policy match. astral-sh/uv#7892 and astral-sh/uv#13954 explain the separate requirement for explicit Python patch upgrades.

## Draft response

This is the same behavior discussed in astral-sh/uv#13167: `uv python list` intentionally includes downloadable versions by default only when automatic downloads are enabled. With `python-downloads = "manual"`, use `uv python list --only-downloads` to see downloads, or `uv python list --allow-python-downloads` to see them alongside installed versions.

Your existing Python 3.14.7 satisfies `uv python install 3` and `uv python install 3.14`; use `uv python install 3.14 --upgrade` to request the latest available 3.14 patch. Your successful explicit installation confirms the build is available. Please continue the request to change or clarify the listing behavior in astral-sh/uv#13167.

## Classification

astral-sh/uv#13167 already covers the same manual-download listing behavior and request for clearer documentation or changed visibility. Its closure was an intentional design decision, not a bug fix, so this is not an established regression. Current source and maintainer comments confirm the policy. The separate install result is expected matching-version behavior, and the reporter confirms that --upgrade works.

The current issue carries enhancement and needs-decision labels, but the recommended triage classification is duplicate because the earlier discussion covers the same requested change and documentation clarification. The architecture and patch release provide a specific example of that policy rather than a distinct mechanism. The earlier issue was closed as NOT_PLANNED on 2025-05-13, with no closing pull request or commit; there is no evidence that a fix to this behavior regressed.

## Related

- astral-sh/uv#13167 — `python-downloads` option affects `uv python list` (issue, closed). Exact match: manual or never downloads hides uninstalled versions, with the same request to show them or clarify the documentation. A maintainer confirmed the behavior is intentional and suggested --allow-python-downloads. Closed as not planned, without a fixing change.
- astral-sh/uv#7892 — [feat] `uv python upgrade` or `uv python install --upgrade` (issue, closed). Covers the separate install expectation: an existing matching Python satisfies a minor-version install request even when a newer patch is available. Maintainer comments distinguish installation from explicit upgrading; it does not address manual-download listing.
- astral-sh/uv#13954 — Support transparent Python patch version upgrades (pull request, merged). Merged on 2025-06-20 and closed astral-sh/uv#7892 and astral-sh/uv#9031 by adding explicit Python upgrade support. Historical context for the install symptom; it did not fix or change the intentional manual-download listing policy.

## Supporting evidence

- In astral-sh/uv#13167, the original report describes both manual and never settings hiding downloadable versions and asks for changed listing behavior or documentation. On 2025-05-06, maintainer zanieb states: “This is intentional. We don't want to show uninstalled versions as ‘available’ when downloads are not automatic.” The comment supplies `--allow-python-downloads` and suggests a possible clearer alias. The issue was closed without implementation.
- In astral-sh/uv#22184, the reporter's 2026-10-05 follow-up confirms both listing overrides and the explicit upgrade command. A same-day maintainer comment explains that downloadable versions are shown when automatic downloads make them transparently available, and that `--only-downloads` explicitly opts in.
- `crates/uv-python-commands/src/list.rs:98` gates default download enumeration on `python_downloads.is_automatic()`. The Downloads branch at line 89 bypasses that condition. The code contains an explicit comment explaining why nonautomatic downloads are hidden.
- `crates/uv-cli/src/settings.rs:1465` selects the Downloads listing mode for `--only-downloads`. `crates/uv/tests/python/python_list.rs:473` contains an existing snapshot test that explicitly removes the test context's disabled-download setting to exercise the default downloadable-version display.
- `crates/uv-python-commands/src/install.rs:513` treats an existing matching installation as satisfying an ordinary request. The upgrade branch instead requires the selected download version/build. `crates/uv-cli/src/lib.rs:6117` documents that patch upgrades require `--upgrade`, with a minor-version request.
- The checkout's download metadata includes `cpython-3.14.8-darwin-x86_64-none` at `crates/uv-python-managed/download-metadata.json:20453`. This checkout has newer metadata than the reported uv release, so it is not independent proof of the old release's contents; the reporter's successful installation and listing overrides establish availability in their environment.
- The closure timeline for astral-sh/uv#7892 identifies astral-sh/uv#13954, merged on 2025-06-20. This predates the October 2026 report and provides explicit-upgrade history, not a listing-policy fix.

## Candidates ruled out

- astral-sh/uv#18913 reported absent Python 3.14.4 download metadata in uv 0.11.4. Exact-version installation failed. Maintainers identified astral-sh/uv#18917 as the fix and announced release in uv 0.11.5. Both the issue comments and merged PR were inspected. Here, exact-version installation succeeds and listing changes with the download-policy override, so this is not evidence of that bug recurring.
- astral-sh/uv#19788 concerns Python release availability and release scheduling. Its example fails to install an exact new version; the 3.14.8 mention is part of a release calendar. Its maintainer comments describe release timing, not manual-download visibility.
- astral-sh/uv#11551 concerns silent skips and unclear output from `uv python install`. This reporter receives an explicit “already installed” message. The linked merged astral-sh/uv#12124 fixes `--reinstall` for an uninstalled version, a different trigger.
- astral-sh/uv#9031 provides additional evidence that ordinary install requests should not implicitly upgrade. Its discussion points to astral-sh/uv#7892, and astral-sh/uv#13954 closed both; the related list uses that canonical upgrade history.

## Search scope and limitations

Separated hidden downloadable versions from install requests retaining an existing matching Python. Searched open/closed issues with authenticated gh GraphQL using 3.14.8, python-downloads/manual, uv python list/missing, already installed, downloadable, hide, automatic, and show-python-downloads; removed version/platform restrictions for conceptual matches. Queried open/closed/merged PRs for listing, manual downloads, version support, and patch upgrades, and followed issue comments and closure timelines to merged PRs. REST search was rate-limited and PR keyword searches returned no matches, limiting exhaustive PR coverage; linked PRs and 100 recent PRs were inspected directly. Ruled out astral-sh/uv#18913 and its merged fix astral-sh/uv#18917: missing metadata prevented exact-version installation, unlike this report. Also ruled out release timing in astral-sh/uv#19788 and silent-install messaging in astral-sh/uv#11551.

Literal searches covered the exact Python release, configuration setting, commands, and “already installed” output. Conceptual searches covered downloadable-version visibility, automatic-download policy, and explicit patch upgrades, informed by documentation and area:cli discussions. Searches for missing metadata and merged upgrade work were kept separate from symptom searches. Issue comments and closure timelines were inspected to distinguish intentional closure from a historical fix.

PR search results did not provide reliable comprehensive coverage: GraphQL search queries requesting PRs returned issue nodes, and `gh pr list --state all --search` returned empty results. Direct PR reads and an all-state recent-PR listing were available. No claim of an exhaustive absence of other PRs is made.

This handoff is based on source inspection and GitHub discussion, not a fresh macOS reproduction. No builds or integration tests were run. Only this temporary issue-context README was authored; no checkout files or GitHub state were changed.
