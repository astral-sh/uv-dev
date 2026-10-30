# Annotated-tag object id given as a git rev fails with "Exact Git revision does not match precise commit"

Issue: astral-sh/uv#23018

Classification: bug

## Summary

On uv 0.13.6 (Homebrew, macOS/aarch64), the reporter passes the full 40-character object ID of an annotated Git tag as the revision in a Git dependency URL used by `uvx --no-cache --from`. Git resolves that object to its target commit, after which uv rejects the differing IDs with “Exact Git revision ... does not match precise commit ...”. Pinning the target commit succeeds according to the report.

The report requests either accepting and peeling annotated-tag object IDs or explaining their rejection with a commit-pin workaround, plus documentation of the distinction. The rejection is intentional and has an existing integration test, including in 0.13.6. The remaining correctness issue is the misleading diagnostic. No duplicate tracking that correction was identified.

astral-sh/uv#22466 explicitly tests the reported rejection, building on exact-commit validation from astral-sh/uv#21224. Neither addresses the requested annotated-tag explanation; no duplicate was found.

## Draft response

A full SHA revision currently has to identify a commit. astral-sh/uv#21224 added that validation, and astral-sh/uv#22466 explicitly tests rejection of annotated-tag object IDs. The two IDs you show identify the tag object and its target commit, respectively.

The error should explain that distinction and suggest the commit ID. For now, use the SHA returned by `git rev-parse 'v2.32.3^{commit}'` in a clone, or reference the tag by name with `@v2.32.3`. A focused next step is an annotated-tag-specific diagnostic and a documentation example; accepting tag-object IDs would require revisiting the exact-commit policy.

## Classification

The rejection is intentional and already tested, but the diagnostic misleadingly presents a tag-object-versus-commit distinction as a generic revision mismatch without explaining the object type or remedy. This is a diagnostic correctness bug; accepting tag-object IDs would separately change the exact-commit policy. The related PRs establish current behavior rather than track this correction, and no previously fixed annotated-tag bug was found to have regressed.

The existence of a test expecting rejection does not resolve the diagnostic complaint. The source confirms the object-ID classification, peeling, and equality check; it does not establish that annotated-tag IDs should become accepted commit pins. Keep the diagnostic correction separate from that policy decision.

The two related changes predate the reported release and deliberately establish the current rule. There is no evidence here of a previously fixed annotated-tag bug returning.

## Related

- astral-sh/uv#22466 — Represent full Git revisions as commit references (merged). Parses full hexadecimal revisions as GitReference::Commit and adds install_git_full_commit_rejects_tag_object, which snapshots the exact reported error for fresh and cached annotated-tag objects. Merged October 14, 2026; the test is present in 0.13.6. Establishes intentional rejection, but does not address the requested explanatory diagnostic or documentation.
- astral-sh/uv#21224 — Ensure full Git commit pins resolve to the requested commit (merged). Introduced exact-object resolution and requested-versus-resolved commit validation to prevent SHA-shaped branches from overriding commit pins. Merged August 19, 2026 and referenced by astral-sh/uv#22466. Explains the validation constraint; its branch-substitution fix does not resolve annotated-tag diagnostic clarity.

## Supporting evidence

- `crates/uv-git-types/src/reference.rs:42`: `GitReference::from_rev` parses a full object ID as `GitReference::Commit`.
- `crates/uv-git/src/git.rs:145`: the Git reference resolver runs `rev-parse <object>^0` for a commit reference. The source comment explains that `^0` peels tags and verifies a commit target.
- `crates/uv-git/src/source.rs:125`: the fetched revision is passed to `GitUrl::with_precise` before checkout.
- `crates/uv-git-types/src/lib.rs:171`: `with_precise` rejects a commit reference whose ID differs from the resolved ID; the error definition at line 88 contains neither an annotated-tag explanation nor a workaround.
- `crates/uv/tests/pip_install/pip_install.rs:3386`: `install_git_full_commit_rejects_tag_object` creates an annotated tag and expects this exact error on both the first fetch and the subsequent cached attempt. It is gated by `test-git`. The test already covers the reported trigger, so a diagnostic change should update this focused coverage rather than duplicate it.
- The GitHub contents API confirms that the same test exists in the `0.13.6` tag. astral-sh/uv#22466 merged on 2026-10-14 and astral-sh/uv#21224 merged on 2026-08-19, before the reporter's October 26 build.
- The public GitHub Git-data API confirms that `psf/requests` object `61e2240f283f15780ac2d0e2cfefb0fd6fdab627` is annotated tag `v2.32.3`, pointing to a commit object with ID `0e322af87745eff34caffe4df68456ebc20d9068`. The mismatch is between different object types for the same tagged revision.
- `docs/concepts/projects/dependencies.md:281` documents named tags and revisions (“commit”), but its Git section does not explain the annotated-tag object distinction or `git rev-parse '<tag>^{commit}'`.
- The new issue has no comments. Related PR bodies, issue comments, PR discussion comments, and review summaries were inspected. Neither related PR proposes the explanatory diagnostic requested here.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered the exact error, annotated-tag/tag-object terminology, the reported object ID, rev-parse, and the existing test identifier. Conceptual searches covered peeling, commit-ish references, full/40-character SHA pins, tag/hash ambiguity, Git diagnostics, and commit-pin documentation, including area:sources, area:error-messages, and documentation labels. Fix-oriented investigation followed Git source history and PR references, inspected both related PRs and their comments/reviews, and checked the 0.13.6 test. PR text searches returned no matches even for known relevant PRs, so coverage was supplemented by the latest 1,500 PRs across all states and file commit history. Ruled out astral-sh/uv#10779 (hexadecimal tag names, not tag-object IDs), astral-sh/uv#8050 (inconsistent lockfile commits), astral-sh/uv#16303 and astral-sh/uv#9045 (missing Git failure details, not this explicit mismatch), and astral-sh/uv#8490 (basic rev syntax). No existing item was found tracking the requested annotated-tag diagnostic correction.

The report was decomposed before searching into three claims: failure when a full SHA identifies an annotated-tag object, a misleading mismatch error, and missing commit-pin guidance. Searches for observable failures and documentation were kept separate from source-mechanism searches.

Additional inspected candidates were astral-sh/uv#2614 (tag and commit URLs conflicting across dependencies), astral-sh/uv#6336 (short revisions causing stale-lock errors), astral-sh/uv#5860 (a precise-commit locking panic), astral-sh/uv#2673 (duplicate Git lookups), astral-sh/uv#298 (branch/tag name precedence), astral-sh/uv#7866 (dynamic-version metadata after tag changes), and astral-sh/uv#7919 (newline handling in commit IDs). Their triggers and failures differ from rejection of an annotated-tag object ID.

The discussion chain from astral-sh/uv#16303 to astral-sh/uv#12713 concerns hidden Git failures and subdirectory/private-repository cases, not an explicitly reported tag-to-commit mismatch. astral-sh/uv#3368 concerns introducing more precise Git reference variants, without tracking this diagnostic.

GitHub search briefly hit its search-rate limit; later searches succeeded. PR text-search results omitted known merged PRs, so the direct PR listing and commit-history follow-up are material limits on any claim of exhaustive coverage. A supplemental search in astral-sh/uv-dev yielded no relevant result.

## Recommended follow-up

Improve the diagnostic for a verified annotated-tag object and show how to obtain and pin its target commit. Do not describe every requested/resolved-ID mismatch as an annotated tag: genuine mismatched commits must remain distinguishable. Add a short Git dependency documentation example using `git rev-parse '<tag>^{commit}'`.

If accepting tag-object IDs is considered instead, explicitly evaluate it against the exact-commit validation introduced by astral-sh/uv#21224. Update the existing annotated-tag integration snapshots and retain checks for mismatched commits and SHA-shaped branch names.

## Validation and limits

Findings are based on source inspection, existing test snapshots, PR history, and independent verification of the public tag object. The reporter's uv command was not executed, and no build or test suite was run. No checkout files or GitHub objects were changed; this handoff is the only authored artifact.
