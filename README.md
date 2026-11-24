# pep508_rs 0.9.2: Requirement::from_str panics on a package name ending in '-' followed by more input

Issue: astral-sh/uv#23388

Classification: `duplicate`

## Summary

In the standalone `pep508_rs` 0.9.2 crate, `Requirement::from_str` panics on a name token ending in `-` when a specifier, extras, a URL, a marker, or whitespace follows. A bare `D-` returns `Err(Pep508Error)` as expected. astral-sh/uv#13102 is the same name-token panic; astral-sh/uv#13105 fixed uv, but not the older standalone `pep508_rs` 0.9.2 release.

## Draft response

You’re hitting the same package-name parser panic tracked in astral-sh/uv#13102. The fix in astral-sh/uv#13105 shipped in uv 0.6.17 and makes the parser return an error for inputs like `D-==1`. The separately published `pep508_rs` 0.9.2 predates that fix and has no newer release. If your integration can use an unstable Rust API, try the `uv-pep508` crate, which includes the fix.

## Classification

The released 0.9.2 source and the pre-fix uv parser both check the final punctuation only at end of input, then assume package-name validation succeeds. The existing issue and fix cover the same bug. The affected standalone crate was published before the uv fix and never received it in a release, so this is not a regression.

## Related

- astral-sh/uv#13102 (closed) — `uv pip install` panicked with `InvalidNameError("wheel-")` when PowerShell passed `wheel-*.whl` literally. The shell and suffix differ, but the parser reached the same invalid name token followed by more input. The maintainer linked the parser fix.
- astral-sh/uv#13105 (merged) — Fixes astral-sh/uv#13102 by validating the last character of the name token regardless of what follows, before constructing `PackageName`. The regression test expects a parse error for `wheel-*.whl`, and the same check covers the reported suffixes.

## Downstream impact

- The reporter found malformed entries in public `requirements.txt` files, `tensorflow-==2.15.0` and `djangorestframework-==.14.0`. Both reportedly trigger `InvalidNameError` panics in `pep508_rs` 0.9.2, so the trigger is not limited to synthetic or fuzzed inputs.
- The reporter also observed `maturin build` aborting with Maturin 1.10.2 when `pyproject.toml` contained `dependencies = ["tensorflow-==2.15.0"]`. The `PyO3/maturin` `v1.10.2` `Cargo.lock` confirms that release resolves `pep508_rs` to 0.9.2; the build failure was not independently reproduced.
- The reporter observed a normal parse error with uv 0.13.7. That is consistent with the existing uv fix and does not indicate a regression in uv.

## Supporting evidence

- The published `pep508_rs` 0.9.2 source (`src/lib.rs`, `parse_name`) rejects `-` as the last character only when `cursor.peek().is_none()`. With `D-==1`, the `=` ends the name token without triggering that check. `PackageName::new` rejects `D-`, and the subsequent `expect` panics. The same check does run for bare `D-`.
- The crates.io index records `pep508_rs` 0.9.2 on January 2, 2025, with no newer release. The uv fix merged on April 25, 2025, and the uv 0.6.17 release notes list it. The separately published `uv-pep508` crate includes the fix from 0.0.1; its Rust API is internal and unstable under `docs/reference/policies/versioning.md`. The standalone project documents that it is no longer actively maintained in konstin/pep508_rs#25.

## Search scope

Searched open and closed issues and open, closed, and merged PRs using the exact panic and error fragments, `Requirement::from_str`, `D-==1`, and `pep508_rs` 0.9.2, then broader searches for trailing punctuation, invalid package names, PEP 508 parser panics, and package publication. Followed the original report’s maintainer comment to the merged fix and checked its diff and release. Ruled out astral-sh/uv#8281 and astral-sh/uv#8282 (leading-whitespace cursor bounds), astral-sh/uv#23091 (deep marker nesting), and astral-sh/uv#3538 (crate synchronization rather than a parser panic).
