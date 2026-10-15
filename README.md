# license-file globs find non-license files and complain about encoding

Issue: astral-sh/uv#22579

Classification: bug

## Summary

No duplicate identified. astral-sh/uv#16693 and astral-sh/uv#16697 cover unmatched license globs; astral-sh/uv#16751 and astral-sh/uv#18772 concern related directory-matching semantics.

The reported failure is supported by the current source: directory-prefix acceptance is used to select regular license files, allowing unrelated files to reach UTF-8 validation.

## Draft response

Your example exposes a bug in license-file selection. The backend uses a directory-prefix check when collecting license files, so `**/LICENSE.txt` can admit `thing.bin` even though its filename does not match. That file then reaches the license UTF-8 check. File selection needs to require a full glob match while retaining prefix checks for directory traversal.

For now, you can list each license path explicitly, including any nested vendored licenses. You do not need to re-encode or relocate the binary: files under the Python package are included in the wheel by default. A vendored license remaining beside the code and also being copied into `.dist-info/licenses` is expected.

## Classification

Source inspection confirms that license_metadata collects regular files using GlobDirFilter::match_path, which accepts directory-prefix matches as well as full glob matches. With **/LICENSE.txt, a nonmatching binary can therefore be collected and subjected to license UTF-8 validation. The related issues and PRs address different behavior; no duplicate or previously fixed version of this failure was established. Inclusion of data files within the Python package is expected.

## Related

- astral-sh/uv#16693 — uv build does not warn or fail when license files do not exist (closed issue). Related license-glob validation history: unmatched patterns silently produced empty license directories. This report instead selects an existing file that does not match the pattern, so it is a different correctness problem.
- astral-sh/uv#16697 — Error when a `project.license-files` glob matches nothing (merged pull request). Fixed astral-sh/uv#16693 on 2025-11-14 by checking that each license glob matches a file. Its diff retains the existing match_path-based file collection; the exact matchers only track unmatched patterns. It does not fix the false-positive selection reported here.
- astral-sh/uv#16751 — Build backend: `source-include` silently ignores directory without a glob pattern (open issue). Discusses the same GlobDirFilter and the distinction between directory traversal and inclusion. Its requested behavior concerns including children of explicitly selected directories, whereas this report concerns incorrectly treating nonmatching regular files as licenses.
- astral-sh/uv#18772 — Don't silently ignore directories in source-include (open pull request). Open work for astral-sh/uv#16751 changes the shared directory matcher to include descendants of matched directories. The inspected diff retains match_path's directory-prefix acceptance and does not address this license-file false positive; relevant for coordinating changes to the matcher.

## Report and scope

The reporter is experimenting with migrating pip to uv_build. Their minimal project has a top-level LICENSE.txt, src/zzz/__init__.py, a vendored LICENSE.txt, and src/zzz/_vendor/thing.bin containing the single byte 0xff. The configured license patterns are `LICENSE.txt` and `src/zzz/_vendor/**/LICENSE.txt`.

Running `uv build --wheel` fails with `Invalid project metadata` and `License file src/zzz/_vendor/thing.bin must be UTF-8 encoded`. Removing the recursive component avoids the error. The report also asks implicitly about retaining package data beside code and seeing vendored licenses both in the package and under .dist-info/licenses.

The reported environment is Linux Mint 22.3, Python 3.12.3, and uv 0.11.25. The build-system requirement is `uv_build>=0.13.1,<0.14`; the CLI version and backend requirement are distinct, and the exact resolved backend version is not supplied. This does not prevent classification because the checkout contains the faulty selection logic.

The claims were evaluated separately:

- Recursive license globs incorrectly select nonmatching files, causing binary resources to fail UTF-8 validation.
- Package data under the importable package remains in the wheel; setting build-backend.data does not resolve license selection.
- Vendored license files remain beside code while also being copied into distribution license metadata.

## Supporting evidence

Inspected checkout commit: `91122e71cc5a9d213225c33299c4914e4a08e0d1`.

- `crates/uv-globfilter/src/glob_dir_filter.rs:78`: `match_path` returns `match_directory(path) || glob_set.is_match(path)`. The directory check intentionally allows prefixes for which some descendant could match; it does not inspect filesystem type. With the reported recursive pattern, a hypothetical descendant named LICENSE.txt beneath thing.bin would match, even though thing.bin is actually a file.
- `crates/uv-build-backend/src/metadata.rs:786`: license discovery uses this directory filter during traversal, then calls `match_path` at line 808 before accepting regular files or symlinks. The per-pattern exact matchers at line 827 only track whether each glob has any match; they do not prevent the unconditional addition to license_files at line 840. Lines 855–859 then validate every collected file as UTF-8.
- `crates/uv-build-backend/src/wheel.rs:544`: `wheel_subdir_from_globs` uses the same traversal and selection methods when copying licenses. A fix should consider both metadata selection and archive contents.
- `docs/concepts/build-backend.md:221`: wheels include the package module, copy license files into .dist-info, and separately copy configured data directories into .data. The following paragraphs explicitly allow data files alongside source code. Retaining thing.bin in the package and copying a vendored license into .dist-info/licenses are consistent with these rules.
- The maintainer response in astral-sh/uv#16596 likewise recommends putting configuration/data files inside the package directory; its original report concerns a file outside that directory and is not this bug.
- The diff and review of astral-sh/uv#16697 show that its November 2025 fix added per-glob missing-match checks while retaining the pre-existing file-collection logic. This is not evidence of a regression of that fix. No release fixing this false-positive selection was identified.
- The diff and comments of astral-sh/uv#18772 concern recursively including actual matched directories. That open work changes the shared matcher, but retains the prefix-accepting file-selection method and does not establish a fix for this report.

These findings come from source, documentation, and discussion inspection. No build or runtime reproduction was performed.

## Suggested follow-up

Use full glob matching to decide whether a regular file is a license, while allowing prefix matching to prune directory traversal. Review the shared callers and coordinate with astral-sh/uv#18772 without treating that work as a duplicate.

Existing tests in `crates/uv/tests/build/build_backend.rs` cover unmatched license globs (`license_glob_without_matches_errors`, line 1009), explicitly selected non-UTF-8 license files (`license_file_must_be_utf8`, line 1051), and symlinked licenses. None of those scenarios covers a recursive license pattern alongside an unrelated binary resource. A focused regression test should verify successful building, retention of the binary in the package, and exclusion of that binary from license metadata and .dist-info/licenses, while retaining the real vendored license.

Use the existing explicit fixture and uv_snapshot style. The build_backend module is already gated by `test-python` in `crates/uv/tests/build/main.rs:12`; assess any additional feature requirement against the chosen test setup. No test or implementation changes are part of this handoff.

## Search coverage and limits

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs with authenticated gh. Separated recursive license selection/UTF-8 rejection from package-data placement and license copying. Literal queries included 'must be UTF-8 encoded', 'license-files', 'license glob', 'non-license', and '0.13.1'; conceptual queries covered recursive matching, unrelated/extra files, vendored licenses, package data, and area:build-backend. Separate mechanism searches used GlobDirFilter and match_directory. Inspected candidate comments, timelines, PR reviews, and diffs. PR keyword searches returned no results despite timeline-linked PRs being retrievable, limiting search completeness. Followed astral-sh/uv#16693 to merged astral-sh/uv#16697 and astral-sh/uv#16751 to open astral-sh/uv#18772. Ruled out astral-sh/uv#15040 (default license discovery), astral-sh/uv#13280 (escaping unsupported glob characters), and astral-sh/uv#15655 (wrong backend); astral-sh/uv#16596 clarifies package-data placement but does not track this failure.

No existing canonical discussion of the same failure was identified. Because keyword PR searches did not surface even the independently verified historical PR, absence of further matches should not be treated as exhaustive proof.

## Handoff status

Only this issue-context README was updated. Repository files and GitHub were not changed. The proposed response is for maintainer review and has not been posted.
