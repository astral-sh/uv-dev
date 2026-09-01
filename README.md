# Should `uv workspace list --scripts` be resilient to malformed scripts?

Issue: astral-sh/uv#21398

Classification: enhancement

## Summary

`uv workspace list --scripts` aborts discovery when any candidate Python file contains invalid PEP 723 metadata. The reported Ruff workspace contains an intentionally invalid fixture with two metadata blocks, so one fixture prevents the command from listing every otherwise valid script.

The strict behavior is established rather than accidental. astral-sh/uv#20784 changed the shared discovery path so `uv check` ignores malformed discovered metadata, but explicitly kept `uv workspace list --scripts` strict and added an integration test for the distinction. No existing issue or pull request was found that tracks changing the listing command to warn and skip invalid candidates or adding a `--skip-invalid` mode. The closest history explains the current behavior and the exact duplicate-block error, but does not duplicate this request.

## Draft response

`uv workspace list --scripts` currently treats malformed candidate metadata as an error. This distinction is explicit in astral-sh/uv#20784: `uv check` skips malformed discovered metadata, while explicit script checks and `uv workspace list --scripts` remain strict.

Since `--scripts` is still a preview feature, changing discovery to warn and skip malformed candidates is reasonable to consider. An opt-in `--skip-invalid` flag would preserve strict behavior but add interface surface. The next step is to decide the default semantics here before implementation.

## Classification

This is an enhancement. The report asks to make an existing preview command more resilient or to add an opt-in mode. Current source collects every discovery result and returns an error on the first malformed candidate, and the `workspace_list_scripts_invalid_metadata` integration test snapshots that failure. astral-sh/uv#20784 deliberately introduced per-candidate errors so consumers can choose a policy; it made `uv check` tolerant while retaining strict behavior for `uv workspace list --scripts`. That evidence establishes intentional current behavior, not a regression or an already-tracked correctness bug.

The parser's rejection of two complete PEP 723 blocks is itself correct: astral-sh/uv#18617 identified prior acceptance as incompatible with PEP 723, and astral-sh/uv#19544 implemented the exact error seen here. The open design question is how a workspace-wide listing operation should handle that valid parse failure.

## Related

- astral-sh/uv#20784 — “Ignore malformed PEP 723 scripts during project checks” (merged pull request). This is the closest precedent: it covers malformed PEP 723 fixtures discovered under a workspace and shows the same Ruff path and duplicate-block error. It changed `uv check` to continue, but explicitly retained strict behavior for `uv workspace list --scripts` and added the current integration test.
- astral-sh/uv#20009 — “Add `uv workspace list --scripts`” (merged pull request). This introduced the preview feature for enumerating standalone scripts and framed discovery as behavior to ship and iterate on, but did not decide how malformed candidates should be handled.
- astral-sh/uv#18617 — “Script metadata parser silently accepts duplicate script blocks” (closed issue). This is the canonical report for the exact malformed condition. It establishes that duplicate PEP 723 blocks must be rejected, but does not address whether workspace-wide discovery should stop or skip.
- astral-sh/uv#19544 — “Reject duplicate PEP 723 script blocks” (merged pull request). This closed astral-sh/uv#18617 and added the exact `The script contains multiple PEP 723 metadata blocks` failure. It explains why the Ruff fixture is invalid; changing list traversal would not change parser correctness.

## Search evidence

Searched open and closed issues and open, closed, and merged pull requests for the literal command and feature identifiers (`uv workspace list --scripts`, `workspace-list-scripts`), exact error fragments (`Failed to discover PEP 723 scripts`, `multiple PEP 723 metadata blocks`), malformed or invalid PEP 723 metadata, and proposed `warn`, `skip`, `skip-invalid`, and resilient-discovery behavior. Conceptual searches covered script enumeration, workspace traversal, candidate parse failures, and cause-neutral discovery failures. Fix-oriented review followed the command's file history through astral-sh/uv#20009, astral-sh/uv#20099, astral-sh/uv#20676, and astral-sh/uv#20784, and followed the duplicate-block report to astral-sh/uv#19544.

astral-sh/uv#20055 was inspected as a same-command candidate but ruled out because it concerns whether the root project appears among workspace members, not PEP 723 discovery or error tolerance. astral-sh/uv#18215 concerns missing workspace-command documentation and is likewise not behaviorally related. No same-request tracker or prior fix for tolerant `uv workspace list --scripts` behavior was found.
