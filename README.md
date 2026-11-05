# `uv pip install ./file` fails when filename contains `#`

Issue: astral-sh/uv#23118

Classification: bug

## Summary

The reporter uses uv 0.12.22 (70fe1196a, 2026-10-01) with Python 3.14.0 on Arch Linux and reports two failures that pip does not exhibit:

1. `uv pip install ./aprict-0.0.38-cupy-issue-#10458.tar.gz` reports `Distribution not found at: file:///home/murdock/cupy-issue-%2310458/aprict-0.0.38-cupy-issue-#10458.tar.gz`. The literal `#10458` belongs to the archive filename. The working directory also contains a `#`, which appears percent-encoded in the diagnostic.
2. Renaming the archive to `aprict-0.0.38-cupy-issue-10458.tar.gz` reaches the build but fails with `Package metadata name aprict does not match given name aprict-0-0-38-cupy-issue`.

astral-sh/uv#4917 and astral-sh/uv#4928 explain the related GitHub-archive name-inference fix; astral-sh/uv#2501 and astral-sh/uv#2502 explain intentional path-fragment handling. Neither pair covers a literal # in a local archive filename.

## Draft response

The path parser currently treats the first `#` in a supplied path as a URL fragment, so it looks for a truncated filename. Removing `#` exposes a separate problem: uv infers `aprict-0-0-38-cupy-issue` from the archive filename. The fix for astral-sh/uv#4917 in astral-sh/uv#4928 applies specifically to GitHub archive URLs, so it does not cover this local archive.

As a workaround, try supplying the package name and an absolute file URL: `uv pip install 'aprict @ file:///absolute/path/aprict-0.0.38-cupy-issue-%2310458.tar.gz'`. Replace `/absolute/path` with your directory and encode any `#` there as `%23` too. This should avoid both parsing problems, though it has not been verified against your archive.

## Classification

The checkout's unnamed local-path parser splits at the first # before filesystem lookup, treating part of the actual filename as a URL fragment; this supports the reported incorrect missing-distribution error. Separately, the renamed archive is parsed at its final hyphen, producing the reported incorrect inferred package name. The historical fixes address intentional URL fragments and GitHub archive URLs, respectively; neither establishes that this local-file behavior was previously fixed. No existing canonical issue or pull request covering this case was found.

The report is actionable as a correctness problem without requiring reproduction first. The metadata-name mismatch is a separate failure exposed by the reporter's attempted workaround. The supplied archive filename is nonstandard, but that does not establish that interpreting a literal filesystem character as a URL fragment is correct.

This is not classified as a duplicate of astral-sh/uv#4917: that issue concerned GitHub-generated remote archive filenames, and its merged fix explicitly excludes those URLs from name inference. The current source retains that exception. There is no evidence that the fix previously covered this local-file case, so the report should not be described as a confirmed regression. Likewise, astral-sh/uv#2501 asked for intentional fragment interpretation, whereas this report needs a literal filename character.

## Related

- astral-sh/uv#4917 (issue, closed): `uv pip install https://...zip` fails where `pip install` works. The same metadata-name mismatch occurred when a GitHub archive filename was mistaken for a distribution name. A maintainer confirmed the bug; astral-sh/uv#4928 fixed GitHub archive URLs specifically. The new report concerns a local archive and additionally a literal # in its filename, so this is historical context rather than an established regression.
- astral-sh/uv#4928 (pull request, merged): Avoid inferring package name for GitHub Archives. Merged on 2024-07-09 to fix astral-sh/uv#4917. Its diff skips filename-based name inference only for github.com URLs containing an archive path segment. That exception remains in the checkout and does not cover the reported local .tar.gz file.
- astral-sh/uv#2501 (issue, closed): uv fails to install wheel from path with fragment. Also produced Distribution not found for a local path containing #, but the reporter intended #hash=somehash as a URL fragment. Here #10458 is part of the actual archive filename: the intended interpretation is the opposite.
- astral-sh/uv#2502 (pull request, merged): Allow direct file path requirements to include fragments. Merged on 2024-03-18 to fix astral-sh/uv#2501 by separating the path at # and restoring the suffix as a URL fragment. The checkout still splits supplied paths this way, explaining the relevant parsing behavior; the PR did not add support for literal # characters in filenames.

## Supporting evidence

- In `crates/uv-pep508/src/unnamed.rs:45`, `UnnamedRequirementUrl::parse_path` delegates to `VerbatimUrl::from_path_with_fragment`.
- In `crates/uv-pep508/src/verbatim_url.rs:223`, `from_path_with_fragment` splits the supplied path before resolving it against the base directory and then restores the suffix as a URL fragment. `split_fragment` at line 746 uses the first `#` without checking whether it belongs to an existing filename. This matches the reported unencoded `#10458.tar.gz` suffix while allowing the base directory's `#` to be encoded.
- In `crates/uv-requirements/src/unnamed.rs:106`, the resolver tries to infer a source archive's name from its filename. Its exception at line 120 is limited to GitHub archive URLs and agrees with the diff of astral-sh/uv#4928.
- In `crates/uv-distribution-filename/src/source_dist.rs:99`, `parsed_normalized_filename` removes the archive extension and splits at the last hyphen. For the renamed archive, that assigns `10458` as the version and `aprict-0.0.38-cupy-issue` as the package name; normalization gives the name shown in the error.
- The astral-sh/uv#4917 timeline identifies astral-sh/uv#4928 as its fixing pull request. The astral-sh/uv#2501 timeline identifies astral-sh/uv#2502 as its fixing pull request. Both merged in 2024, before the reported uv release.
- The proposed workaround uses an explicitly named requirement to avoid filename-based name inference and `%23` in an absolute file URL to represent literal `#` characters. It is source-informed and has not been executed against the reporter's archive.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests with authenticated gh. Decomposed the report into literal-# local-path lookup failure and renamed-source-archive metadata-name mismatch. Literal searches included Distribution not found, does not match given name, metadata name, filename contains, and the cited astral-sh/uv#4917; conceptual and fix-oriented searches covered hash/number sign/pound, fragments, special characters, percent encoding, renamed/local/source archives, sdist filenames, and name inference, removing package, version, and platform details. Inspected candidate bodies, maintainer comments, timelines, and the diffs of astral-sh/uv#4928 and astral-sh/uv#2502. Ruled out astral-sh/uv#7714 (stale PKG-INFO), astral-sh/uv#9743 and astral-sh/uv#2727 (Git subdirectory metadata selection), astral-sh/uv#14345 (commas in wheel RECORD entries), astral-sh/uv#17526 (HTTP Content-Disposition), and astral-sh/uv#19883 (PEP 625 conformance roadmap) as canonical matches. No existing discussion or fix covering the literal-# filename failure was found.

The particularly plausible metadata-mismatch candidates were distinguished through maintainer comments: astral-sh/uv#7714 involved an incorrect name in a directory's `PKG-INFO`; astral-sh/uv#9743 involved metadata generated in the wrong Git checkout location; astral-sh/uv#2727 involved selecting root metadata instead of subdirectory metadata. Their triggers and mechanisms differ from local archive filename inference.

astral-sh/uv#14345 concerns incorrectly escaped commas inside a built wheel's `RECORD`, rather than parsing the installation argument. astral-sh/uv#17526 concerns obtaining filenames from HTTP response headers. astral-sh/uv#19883 tracks stricter source distribution conformance and does not track literal-`#` path handling; its comments also distinguish source archives from conforming sdists.

## Validation and remaining work

The assessment uses the report, current checkout source, historical pull request diffs, and maintainer discussion. The archive was not provided, and no installation commands, builds, or tests were run. The exact reported release was not independently reproduced; no regression point is established.

A follow-up implementation should independently exercise a literal `#` in an archive filename and filename-based name inference for a local source archive, while retaining explicit URL-fragment behavior. The reporter can try the workaround above; an archive or minimal build recipe would allow end-to-end validation if it still fails.

Only this handoff README was updated. No checkout files were modified, and no issue comments, labels, or other GitHub changes were made.
