# LenientRequirements should allow whitespace after the star operator

Issue: astral-sh/uv#21011

Classification: bug

## Summary

The report identifies a gap in uv's mistake-tolerant parsing of third-party package metadata. uv already rewrites invalid ordered-comparison wildcard specifiers such as `>=1.9.*` to `>=1.9`, but the repair's regular expression requires the version to immediately follow the operator. Consequently, a `Requires-Python` value such as `>= 3.5.*` is not repaired. The reporter found this form in many PyPI METADATA files and provided `fastobo` 0.1.0.dev51 as an example.

The repository source supports the report. `crates/uv-pypi-types/src/metadata/metadata_resolver.rs` parses `Requires-Python` through `LenientVersionSpecifiers`. In `crates/uv-pypi-types/src/lenient_requirement.rs`, the ordered-comparison wildcard repair matches `(<=|>=|<|>)(\d+(\.\d+)*)\.\*`, while its tests cover `>=1.9.*` and `>=1.*` only without whitespace. No existing open issue or pull request was found for the whitespace trigger.

## Draft response

Thanks for the concrete corpus data. This is a bug in the existing lenient metadata repair. uv intentionally normalizes ordered-comparison wildcards from third-party metadata: astral-sh/uv#1528 reported the same `Requires-Python: >=3.5.*` form without whitespace, astral-sh/uv#1507 routed index `Requires-Python` values through the lenient parser, and astral-sh/uv#1529 documents the corresponding repair policy for dependency metadata. The current regex and tests only cover inputs where the version immediately follows the operator, so `Requires-Python: >= 3.5.*` falls through even though it is the same malformed specifier. The next step is to extend that repair for operator-version whitespace and add coverage for both `LenientVersionSpecifiers` and `LenientRequirement`, including a compound value such as `>= 3.6.*, <3.8.*`. astral-sh/uv#8326 is about bare `==*` in user-authored requirements and does not cover this case.

## Classification

This is a bug. The affected metadata is itself invalid under the version-specifier rules because ordered comparisons do not support wildcard versions, but uv intentionally has a lenient path for repairing common errors in third-party metadata. The source, tests, and historical fixes establish that ordered-comparison wildcards are within that intended repair behavior. Treating the equivalent whitespace form differently is therefore an uncovered correctness case, not a request for a new subsystem or policy.

The issue is not a duplicate: the searches found no open tracker for whitespace between the comparison operator and wildcard version. It is also not a demonstrated regression. The historical fixes covered forms without that whitespace, and no evidence was found that the reported form worked in an earlier uv release.

## Related issues and pull requests

- astral-sh/uv#1528 — Closed issue, and the closest prior report. NLTK 3.6 carried `Requires-Python: >=3.5.*`, matching the new issue's metadata field and invalid specifier except for the newly reported whitespace. The reporter confirmed that the no-whitespace case worked in uv 0.1.7 after the relevant parser changes landed.
- astral-sh/uv#1507 — Merged pull request that routed `Requires-Python` values from HTML package indexes through `LenientVersionSpecifiers`. It is part of the resolution history for astral-sh/uv#1528 and establishes use of the lenient parser at an important metadata boundary, although the new report points to wheel METADATA rather than an HTML index attribute.
- astral-sh/uv#1529 — Merged pull request for the same ordered-comparison wildcard repair in `Requires-Dist` metadata. It explicitly documents why uv repairs invalid third-party metadata and expanded the regex still present in current source, but did not accept whitespace after the operator.
- astral-sh/uv#1477 — Closed canonical issue for astral-sh/uv#1529. It reports the same `Operator >= cannot be used with a wildcard version specifier` failure and includes a dependency-metadata reproduction. The fixing pull request explicitly distinguishes strict parsing of user-authored requirements from lenient repair of third-party metadata.
- astral-sh/uv#1402 — Closed issue involving invalid `Requires-Python: >=3.*`. It is adjacent rather than identical because the missing case was a major-only wildcard without whitespace, but it led maintainers to extend this same repair mechanism.
- astral-sh/uv#1410 — Merged pull request that expanded the wildcard repair and tests to major-only versions. It provides precedent for covering another common syntactic variant, but did not add operator-version whitespace support.

## Search scope and exclusions

Literal searches covered `LenientRequirements`, `LenientRequirement`, `>= 3.5.*`, `>=1.9.*`, the exact wildcard-operator parser error, and the repair's diagnostic phrase. Conceptual searches covered invalid or lenient `Requires-Python` and dependency metadata, PEP 440 wildcard comparisons, version-specifier whitespace, comparator wildcards, and parser fixups. Fix-oriented searches covered closed issues, merged pull requests, comments and referenced discussions, and the source history of `lenient_requirement.rs`, including the metadata-boundary change in astral-sh/uv#1507. The repository's open and closed issues and its open, closed, and merged pull requests were searched.

astral-sh/uv#8326 was inspected because the reporter cited it, but it is not the same problem: it asks whether bare `==*` should be accepted in user-authored dependencies and was resolved as a standards question. astral-sh/uv#2546 and astral-sh/uv#2550 were also inspected; they concern a later fixup-order failure for a no-whitespace `Requires-Dist` value, not whitespace matching. astral-sh/uv#1464 concerns trailing commas in `Requires-Python`, so it is another leniency case but not meaningfully close to this trigger.

## Supporting evidence

- `LenientVersionSpecifiers` is used for `Requires-Python` in both metadata-resolution paths in `crates/uv-pypi-types/src/metadata/metadata_resolver.rs`.
- The current repair in `crates/uv-pypi-types/src/lenient_requirement.rs` recognizes only versions adjacent to `<=`, `>=`, `<`, or `>`.
- Existing tests establish the expected normalized results for `>=1.9.*` and `>=1.*`, but contain no operator-version whitespace case.
- astral-sh/uv#1528 records the exact no-whitespace `Requires-Python: >=3.5.*` analogue and confirms that it worked after the relevant parser changes landed; astral-sh/uv#1507 put HTML index `Requires-Python` values on the lenient parsing path.
- astral-sh/uv#1529 documents the repository policy: invalid ordered-comparison wildcards from dependency metadata are repaired because end users do not control that metadata, while equivalent user-authored requirements remain strict.
