# `lock-without-metadata`: `uv lock --check` re-resolves when a conflicting group has `pkg` and `pkg[extra]`

Issue: astral-sh/uv#22046

Classification: bug

## Summary

With the preview `lock-without-metadata` feature enabled, a dependency group that participates in
`tool.uv.conflicts` and contains both a base requirement and the same requirement with an extra can
cause `uv lock --check` to reject a lockfile it just created. The freshness check reconstructs two
requested declarations while the metadata-free lock can contain only the declaration with extras.
That mismatch triggers an unnecessary resolution; `--offline` then fails with an empty cache even
though the project inputs have not changed.

Merged astral-sh/uv#21951 fixed the original reproduction where the two declarations had no version
specifiers and added integration coverage for that case. The reporter subsequently provided a
narrower reproduction showing that the failure remains when the declarations have different
specifiers, such as `requests<3` and `requests[socks]==2.32.3`. The issue body now describes
astral-sh/uv#21951 as a partial fix.

The report covers uv 0.12.0 through 0.12.19 for the original case. The follow-up concerns behavior
after astral-sh/uv#21951, but the remaining case has not been independently reproduced in this
handoff. Disabling `lock-without-metadata`, removing the conflict, or avoiding either overlapping
declaration remains a reported workaround.

## Follow-up reproduction

Use the same project scaffolding as the original reproduction, but define the groups and conflict as:

```toml
[dependency-groups]
a = ["requests<3", "requests[socks]==2.32.3"]
b = []

[tool.uv]
preview-features = ["lock-without-metadata"]
conflicts = [[{ group = "a" }, { group = "b" }]]
```

Then run:

```console
uv lock
uv cache clean
uv lock --check --offline -v
```

The reporter observes the same offline resolution failure. The verbose freshness diagnostic shows
two requested dependencies—an unconditional base dependency and an extra-bearing dependency guarded
by the conflict-group marker—but only the extra-bearing dependency in the existing lock:

```text
DEBUG Resolving despite existing lockfile due to mismatched resolved dependencies for: `mre==0.1.0`
  Requested: [requests with no extra and marker true, requests[socks] with the group marker]
  Existing: [requests[socks] with the group marker]
```

The real project has the same shape through group inclusion: a shared group declares
`apache-airflow<4`, while a leaf group pins `apache-airflow[celery,...]==3.3.2`.

## Classification

Bug. `uv lock --check` should accept an unchanged lockfile produced from the same project inputs.
Instead, the remaining reproduction causes a false stale-lock decision and can make offline
validation fail after the cache is cleared.

The earlier duplicate classification is no longer accurate. astral-sh/uv#21951 has merged and fixes
the equal-specifier/base-plus-extra case, but the new reproduction establishes a materially different
remaining trigger involving overlapping declarations with different specifiers. This is best treated
as incomplete coverage of the reported correctness problem, not as a duplicate that can be fully
centralized on the merged pull request.

This is also not evidence that the older astral-sh/uv#18553 fix regressed. That issue's false positive
required `--refresh` and was traced to a mismatch between deserialized and in-memory fork-marker
forms. Merged astral-sh/uv#18612 canonicalized those fork markers before equality checks. The current
diagnostic instead identifies mismatched resolved dependency declarations reconstructed without
`package.metadata`.

## Related

- astral-sh/uv#21951 — Merged pull request, “Normalize requirement declarations in lockfiles.” It
  normalizes semantically equivalent requirement collections during lock satisfaction and added a
  test for a conflicting metadata-free group containing an unversioned base requirement plus the
  same requirement with an extra. The follow-up shows that this was only a partial fix: differing
  specifiers still produce the stale-lock mismatch.
- astral-sh/uv#18553 — Closed issue, “uv lock --check --refresh false positive in workspace with
  [tool.uv.conflicts].” This is the closest historical symptom involving `uv lock --check` and
  conflicts, but it required `--refresh` and had a different confirmed cause: fork-marker
  canonicalization rather than overlapping base-plus-extra requirement declarations.
- astral-sh/uv#18612 — Merged pull request, “Normalize persisted fork markers before lock equality
  checks.” It fixed astral-sh/uv#18553 by canonicalizing fork markers. Its narrower mechanism shows
  why the current report is not a regression of that prior fix.

## Investigation notes

The remaining comparison differs from the case added to astral-sh/uv#21951 in two important ways:
the base declaration has a broad bound, and the extra-bearing declaration has a narrower exact pin.
The verbose output indicates that the unconditional base edge is absent from the lock's reconstructed
resolved dependencies. The comment establishes the observable mismatch but does not confirm whether
the defect is in metadata-free serialization, reconstruction, or semantic comparison; that mechanism
still needs source-level confirmation.

Earlier searches covered the exact “mismatched resolved dependencies” diagnostic,
`lock-without-metadata`, `uv lock --check`, offline/no-cache behavior, conflict splits, requirement
normalization, and extra activation across issues and pull requests. astral-sh/uv#19106,
astral-sh/uv#18015, and astral-sh/uv#14645 concern extra activation during sync or package-level
conflict resolution and remain non-canonical for this stale-lock failure.
