# 0.13.5 missing from the astral-sh/versions manifest: publish-versions failed ("No interpreter found for Python 3.14.9")

Issue: astral-sh/uv#22826

Classification: bug

## Summary

uv 0.13.5 has a published GitHub release, but the versions manifest consumed by setup-uv still ends at 0.13.4. The reporter's setup-uv v10.3.0 invocation on Linux x86_64 cannot find an artifact for the pinned version 0.13.5. Because the version has no manifest record, the omission affects every platform represented by that manifest.

The release run confirms that manifest publication failed at "Update versions"; committing the manifest and creating and merging its PR were skipped. Documentation publishing failed on the same requested Python version. Both jobs installed uv 0.13.4, whose embedded Python catalog lacks 3.14.9.

The closest prior fix is astral-sh/uv#22810, which omitted manifest and documentation publishing. astral-sh/uv#22802 introduced the Python 3.14.9 requirement; astral-sh/uv#22793 explains the separate earlier cancellation.

## Draft response

Confirmed: 0.13.5 is published, but `v1/uv.ndjson` still ends at 0.13.4. Both publishing jobs installed uv 0.13.4 and failed to find Python 3.14.9.

astral-sh/uv#22810 configured the checkout’s Python download metadata for release planning and assembly, but the separate manifest and documentation workflows lack that setting. The next step is to apply that configuration to both workflows and complete publication for 0.13.5. This is separate from the canceled run discussed in astral-sh/uv#22793.

You can temporarily keep setup-uv pinned to 0.13.4 while the manifest is corrected.

## Classification

The published 0.13.5 release is absent from the live versions manifest, and job logs confirm both publishing workflows fail to find Python 3.14.9 with uv 0.13.4. This is incorrect release publication behavior. The merged astral-sh/uv#22810 fixed earlier release stages but omitted these reusable publishing workflows; the evidence shows an incomplete fix across workflows, not that its corrected stages regressed. astral-sh/uv#22793 concerns a separate canceled run, and no existing issue or open PR was found tracking this remaining failure.

The failures are established by the live manifest, workflow source, job conclusions, and filtered interpreter-error logs. No additional reproduction from the reporter is needed to establish the bug. The Python download metadata is present in the checkout; the publishing jobs do not configure their pinned uv to use it.

## Related

- astral-sh/uv#22810 — Use checked-in Python download metadata for releases (pull request, merged). Fixed the same uv 0.13.4/Python 3.14.9 metadata mismatch in release planning, wheel checks, and signed-artifact assembly before the reported run. Its description explains that reusable workflows need their own environment setting. The diff omitted publish-versions.yml and publish-docs.yml, which still fail; this is the closest prior fix, not a completed fix for manifest publication.
- astral-sh/uv#22802 — Update to include Python 3.15.1, 3.14.9, 3.13.17 (pull request, merged). Changed the first entry in .python-versions from 3.14.8 to 3.14.9 and added the new download metadata. It configured several checking workflows to use that metadata but omitted the publishing workflows, which still install uv 0.13.4. This establishes the triggering change rather than an existing fix for the missing uv manifest entry.
- astral-sh/uv#22793 — uv 0.13.5 listed in CHANGELOG but release appears to have been canceled (issue, closed). Concerns the earlier 0.13.5 release attempt. A maintainer confirmed that run was intentionally canceled because of a Python miscompilation. The current report concerns a later run that published the release but failed to publish its manifest and documentation; it is not a duplicate of that cancellation question.

## Supporting evidence

Evidence was checked on 2026-10-22.

- The [0.13.5 GitHub release](https://github.com/astral-sh/uv/releases/tag/0.13.5) was published at 01:35:54 UTC. Reading [v1/uv.ndjson](https://raw.githubusercontent.com/astral-sh/versions/main/v1/uv.ndjson) found 327 version records, including 0.13.4 and none for 0.13.5; the highest version was 0.13.4.
- [Release run 39703477566](https://github.com/astral-sh/uv/actions/runs/39703477566) ran against commit `46776a95d6686ac0160c10611327ce491e6b202f`. Its first attempt completed with failure. The GitHub publication job succeeded.
- [Manifest publication job](https://github.com/astral-sh/uv/actions/runs/39703477566/job/119588017202): "Install uv" succeeded with uv 0.13.4, "Update versions" failed, and "Commit versions", "Create Pull Request", and "Merge Pull Request" were skipped. The relevant log error is `No interpreter found for Python 3.14.9 in virtual environments, managed installations, or search path`.
- [Documentation publication job](https://github.com/astral-sh/uv/actions/runs/39703477566/job/119588017234): "Install uv" succeeded with uv 0.13.4 and "Build docs" failed with `No interpreter found for Python 3.14.9 in managed installations or search path`. This independently confirms the additional documentation failure.
- At the release commit, [publish-versions.yml](https://github.com/astral-sh/uv/blob/46776a95d6686ac0160c10611327ce491e6b202f/.github/workflows/publish-versions.yml#L58) pins uv 0.13.4 and invokes the versions repository's conversion and insertion scripts with `uv run`. [publish-docs.yml](https://github.com/astral-sh/uv/blob/46776a95d6686ac0160c10611327ce491e6b202f/.github/workflows/publish-docs.yml#L81) pins the same uv version and invokes `uv run --only-group docs mkdocs build --strict -f mkdocs.yml`. Neither workflow sets `UV_PYTHON_DOWNLOADS_JSON_URL`.
- The caller's [release.yml](https://github.com/astral-sh/uv/blob/46776a95d6686ac0160c10611327ce491e6b202f/.github/workflows/release.yml#L35) does set `UV_PYTHON_DOWNLOADS_JSON_URL`. astral-sh/uv#22810 explicitly explains that reusable workflows do not inherit that caller environment. Its diff adds the setting to planning, wheel-content checks, and signed-artifact assembly, but not either publishing workflow.
- astral-sh/uv#22802 merged at 22:37:22 UTC on October 21 and changed the first `.python-versions` entry to 3.14.9. The checkout's Python catalog includes that version; the catalog at tag 0.13.4 does not. astral-sh/uv#22810 merged at 00:56:21 UTC on October 22, before this release run started at 01:03:53 UTC.
- In astral-sh/uv#22793, a maintainer confirmed the earlier run was deliberately canceled because of a Python miscompilation. That explanation concerns the previous release attempt, not these post-publication jobs.

## Recommended next step

Configure both reusable publishing workflows to use the checkout's Python download metadata, following astral-sh/uv#22810. A release maintainer can then complete the missing manifest and documentation publication using the corrected interpreter setup. Verify that the manifest contains 0.13.5 and that setup-uv can resolve the pinned version. Simply requesting another reporter reproduction would not address the established workflow failure.

The draft offers pinning setup-uv to 0.13.4 as a temporary workaround. No workflow fix, rerun, or publication has been performed as part of this handoff.

## Search scope and excluded candidates

Used authenticated gh to search open and closed issues and open, closed, and merged PRs separately for the missing manifest/setup-uv failure, interpreter failure, and documentation failure. Literal searches included 0.13.5, uv.ndjson, astral-sh/versions, publish-versions, publish-docs, 3.14.9, and both reported errors. Broader searches covered release publication, missing artifacts, interpreter discovery, Python metadata, and internal:releases; fix-oriented searches covered merged release/metadata changes. PR keyword searches missed known PRs, so supplemented them with 600 recent PRs across all states, an open-PR listing, and publish-versions file history. Inspected candidate bodies, comments, diffs, and referenced fixes. Ruled out astral-sh/uv#18701 (proxy/TLS failure fetching the manifest), astral-sh/uv#18913 and astral-sh/uv#18917 (missing Python catalog entries in a shipped release), and astral-sh/uv#18813 (dry-run dist-manifest synthesis). No existing tracker for the remaining publication failure was found.

Additional same-mechanism fixes inspected were astral-sh/uv#22633 (test and bug-workflow setup), astral-sh/uv#22816 (Cargo lockfile checking), and astral-sh/uv#22820 (code-quality review setup). None edits the affected publishing workflows. astral-sh/uv#22745 fixes an earlier Python 3.14.8 mismatch in signed-release assembly, a different release stage. astral-sh/uv#19788 concerns scheduling uv releases around new Python releases rather than repairing publication of an already released uv version.

The PR-search limitation means the absence of an older matching tracker cannot be established exhaustively. Recent PR listings, open-PR titles, the affected workflow's history, and direct inspection of the strongest candidates did not reveal a tracker or fix for these remaining publishing failures.
