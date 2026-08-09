# LenientRequirements should allow whitespace when fixing star operators with versions

Issue: astral-sh/uv#21011

Classification: bug

## Summary

The reported behavior is reproducible. uv's lenient metadata parsing repairs an invalid ordered-comparison wildcard when the version is adjacent to the operator, such as `Requires-Python: >=3.5.*`, but does not repair the equivalent spelling with whitespace, `Requires-Python: >= 3.5.*`. The latter causes an otherwise installable wheel to be rejected while reading its metadata.

The report gives `fastobo` 0.1.0.dev51 as a real-world example and reports uv 0.12.1 on Darwin 25.5.0 arm64 with Python 3.13.13. The isolated reproduction below confirms the same behavior with a synthetic wheel, avoiding reliance on mutable index data.

## Classification

This is a bug in an existing leniency path, not a demonstrated regression. Ordered comparisons with wildcard versions are invalid, but uv deliberately repairs this common error in third-party metadata. The otherwise identical result changes solely because whitespace occurs between the operator and version.

## Reproduction

Outcome: `reproducible`.

Environment used:

- uv 0.12.3 (`x86_64-unknown-linux-gnu`)
- CPython 3.12.3 at `/usr/bin/python`
- Linux 6.17.0-1020-azure x86_64
- All wheels, targets, and uv caches were under a fresh `/tmp/uv-issue-21011.*` directory.

Two minimal pure-Python wheels were constructed locally. Their metadata differed only in package identity and the whitespace under test:

```text
Metadata-Version: 2.1
Name: badstar
Version: 1.0.0
Requires-Python: >= 3.5.*
```

```text
Metadata-Version: 2.1
Name: controlstar
Version: 1.0.0
Requires-Python: >=3.5.*
```

The relevant commands were:

```console
$ UV_CACHE_DIR="$case_dir/cache-bad" uv pip install \
    --target "$case_dir/target-bad" --no-index \
    "$case_dir/badstar-1.0.0-py3-none-any.whl"
Using CPython 3.12.3 interpreter at: /usr/bin/python
  × Failed to read `badstar @ file:///tmp/uv-issue-21011.../badstar-1.0.0-py3-none-any.whl`
  ├─▶ Couldn't parse metadata of badstar-1.0.0-py3-none-any.whl from badstar @ ...
  ╰─▶ Failed to parse version: Operator >= cannot be used with a wildcard
      version specifier:
      >= 3.5.*
      ^^^^^^^^

$ UV_CACHE_DIR="$case_dir/cache-control" uv pip install \
    --target "$case_dir/target-control" --no-index \
    "$case_dir/controlstar-1.0.0-py3-none-any.whl"
Using CPython 3.12.3 interpreter at: /usr/bin/python
Resolved 1 package in 1ms
Prepared 1 package in 1ms
Installed 1 package in 0.50ms
 + controlstar==1.0.0 (from file:///tmp/uv-issue-21011.../controlstar-1.0.0-py3-none-any.whl)
```

The whitespace fixture exited 1 and the adjacent control exited 0. This directly demonstrates the reported whitespace-sensitive behavior at the wheel `Requires-Python` metadata boundary.

Existing coverage is limited to unit tests in `crates/uv-pypi-types/src/lenient_requirement.rs`: `requirement_greater_than_star` verifies `torch (>=1.9.*)` becomes `torch (>=1.9)`, and `specifier_greater_than_star` verifies `>=1.9.*` and `>=1.*` become `>=1.9` and `>=1`. Those assertions were read and do not include operator-version whitespace. No matching integration test was found under `crates/uv/tests/` or `crates/uv-client/tests/it/`.

## Related

- astral-sh/uv#1528 is the closest prior report: it covers invalid `Requires-Python: >=3.5.*` without whitespace.
- astral-sh/uv#1507 routed index `Requires-Python` values through `LenientVersionSpecifiers`.
- astral-sh/uv#1529 added the ordered-comparison wildcard repair for dependency metadata and documents uv's policy of repairing invalid third-party metadata.
- astral-sh/uv#1477 is the canonical issue for astral-sh/uv#1529 and reports the same wildcard-comparison parser error without the whitespace trigger.
- astral-sh/uv#1402 and astral-sh/uv#1410 cover the related major-only wildcard form, also without operator-version whitespace.
- astral-sh/uv#8326 concerns bare `==*` in user-authored requirements and does not cover this metadata case.

No related report establishes that the whitespace form worked in an earlier uv release, so the observed failure should not be described as a regression.
