# `lock-without-metadata`: `uv lock --check` re-resolves when a conflicting group has `pkg` and `pkg[extra]`

Issue: astral-sh/uv#22046

Classification: duplicate

## Summary

With the preview `lock-without-metadata` feature enabled, a dependency group that participates in
`tool.uv.conflicts` and contains both a base requirement and the same requirement with an extra
causes `uv lock --check` to reject a lockfile it just created. The freshness check reconstructs two
requested declarations, such as `requests` and `requests[socks]`, while the metadata-free lock
represents their combined resolved dependency. The mismatch triggers an unnecessary resolution;
`--offline` then fails with an empty cache even though the lockfile is current. The report reproduces
the behavior from uv 0.12.0 through 0.12.19 and identifies disabling the preview feature or removing
one of the declarations or the conflict as workarounds.

Open astral-sh/uv#21951 is the canonical match. It predates this report and changes lockfile
satisfaction to normalize semantically equivalent requirement collections, including the path used
when `package.metadata` is absent. Its normalizer combines declarations for the same package,
including their extras, before comparison. The exact base-plus-extra/conflicting-group scenario has
also been proposed on that pull request as integration-test coverage.

## Draft response

Thanks for the focused reproduction. This is covered by astral-sh/uv#21951: its lockfile-satisfaction
changes normalize semantically equivalent requirement collections before comparison, including the
missing-metadata path exercised by `lock-without-metadata`. That covers the `pkg` plus `pkg[extra]`
mismatch that makes the unchanged lock appear stale. The exact scenario has also been proposed there
as integration-test coverage, so let's centralize the fix and follow-up on astral-sh/uv#21951. Until
that lands in a release, disabling `lock-without-metadata` or avoiding the duplicate base/extra
declarations remains the available workaround.

## Classification

Duplicate. astral-sh/uv#21951 was opened five days before astral-sh/uv#22046 and tracks the same
underlying comparison problem: syntactically different but semantically equivalent requirement
collections cause an existing lock to be treated as stale. The pull request's implementation applies
`NormalizedRequirements` while validating missing package metadata, and its normalization combines
same-package declarations, markers, version constraints, and extras. That is the path and semantic
difference shown by this reproduction, so discussion and regression coverage can be centralized on
astral-sh/uv#21951.

This is not a regression of the older astral-sh/uv#18553 fix. That issue's false positive required
`--refresh` and was traced to a mismatch between deserialized and in-memory fork-marker forms.
Merged astral-sh/uv#18612 canonicalized those fork markers before equality checks. The new report's
debug output instead identifies mismatched resolved dependencies reconstructed without
`package.metadata`.

## Related

- astral-sh/uv#21951 — Open pull request, “Normalize requirement declarations in lockfiles.” This
  is the canonical match: it normalizes requirement collections during lock satisfaction, including
  the missing-`package.metadata` path, and combines same-package declarations and extras before
  semantic comparison. The reported reproduction has been proposed there as integration-test
  coverage.
- astral-sh/uv#18553 — Closed issue, “uv lock --check --refresh false positive in workspace with
  [tool.uv.conflicts].” This is the closest historical symptom involving `uv lock --check` and
  conflicts, but it required `--refresh` and had a different confirmed cause: fork-marker
  canonicalization rather than base-plus-extra requirement collections.
- astral-sh/uv#18612 — Merged pull request, “Normalize persisted fork markers before lock equality
  checks.” It fixed astral-sh/uv#18553 by canonicalizing fork markers. Its narrower mechanism shows
  why the current report is not a regression of that prior fix.

## Search evidence

Literal searches covered the exact “mismatched resolved dependencies” debug fragment,
`lock-without-metadata`, `uv lock --check`, offline/no-cache failure, conflicts, and the base-plus-extra
identifiers across open and closed issues and open, closed, and merged pull requests. No earlier issue
matched the exact message or full trigger.

Conceptual searches covered stale or out-of-date locks, semantically equivalent requirement
declarations, requirement and marker normalization, conflict splits, and extra activation.
Fix-oriented inspection followed astral-sh/uv#13614 with astral-sh/uv#13635, astral-sh/uv#15869 with
astral-sh/uv#15884, astral-sh/uv#16839 with astral-sh/uv#18116, and astral-sh/uv#18553 with
astral-sh/uv#18612. Those reports concern false markers, inferred-conflict flags, or persisted fork
markers rather than the current metadata-free requirement collection.

The strongest superficially similar candidate was astral-sh/uv#18553, ruled out as the canonical
discussion because of its distinct confirmed mechanism. astral-sh/uv#19106, astral-sh/uv#18015, and
astral-sh/uv#14645 were also inspected and ruled out because they concern extra activation during
sync or package-level conflict resolution, not whether an unchanged lockfile satisfies its inputs.
