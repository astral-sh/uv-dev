# Bug: `uv pip install -r -` fails to parse `pylock.toml` passed via `stdin`

Issue: astral-sh/uv#22598

Classification: duplicate

## Summary

The reporter can install from a named `pylock.toml` file, but supplying the same content through `uv pip install --requirements -` fails with “Couldn't parse requirement in `-`” and `no such comparison operator "="` at `lock-version = "1.0"`. They request an explicit input-format option, such as `--format pylock.toml`, or content-based detection. The report shows CPython 3.14.0 but does not specify a uv version.

astral-sh/uv#13400 is the closest prior discussion of the same pylock stdin parsing limitation. Related fixes distinguish named-file and URL support from stdin and do not establish a regression.

## Pip compatibility findings

Maintainer woodruffw reports that pip does not support `pip install -r -`, citing pypa/pip#7822. They also report that `pip install -r /dev/stdin < pylock.toml` fails similarly to uv: pip selects the requirements parser based on the filename. The `/dev/stdin` command is a comparison case, not a working workaround for pylock input.

The maintainer cites pip's `src/pip/_internal/cli/req_command.py`, lines 361–385, at commit `634a6ec1a5d9dcc2433571cdb2f4c58a4bb29caf` as the parser-selection reference: https://github.com/pypa/pip/blob/634a6ec1a5d9dcc2433571cdb2f4c58a4bb29caf/src/pip/_internal/cli/req_command.py#L361-L385. These are attributed maintainer findings; this handoff update did not independently inspect pip's source or run either command. No pip release version or command output is supplied in the discussion.

The discussion does not establish a working pip pylock-stdin behavior for uv to match. Maintainer woodruffw explicitly distinguishes uv's existing `-r -` support from the proposed additional divergence from pip: detecting the input type from its contents. The compatibility question therefore concerns content-based format selection, not whether uv should read stdin at all.

Maintainer zanieb raises consistency with the rest of uv as a reason to support `-r -`. That remark does not establish endorsement of pylock auto-detection or an input-format flag. uv already reads requirements-format input through `-r -`; selecting the pylock parser for that input remains unresolved, with no final design decision in the supplied discussion. The duplicate classification remains the prior triage assessment; these comments do not confirm that classification or establish a regression.

## Draft response

This is the same stdin format-selection limitation discussed in astral-sh/uv#13400. That report used `uv add`, but the filename ambiguity also applies to `uv pip install`: stdin without inline script metadata is parsed as `requirements.txt`, so `lock-version` produces this error. An explicit input-format option or pylock detection would need a design decision; please continue that proposal in astral-sh/uv#13400. For now, save the stream to a file named `pylock.toml` and run `uv pip install --requirements pylock.toml`.

## Classification

astral-sh/uv#13400 already discusses the same pylock stdin format-selection limitation and identical parser error. Although its reproduction uses uv add, the maintainer explicitly addresses the absence of a filename for identifying stdin's format, and the current shared requirements-source code confirms that limitation for uv pip install. The proposed input-format flag or detection would revisit that discussion. The follow-up fix only rejected unsupported pylock inputs in uv add; no evidence establishes a previously fixed stdin feature or regression.

The duplicate relationship rests on the exact stdin capability, error, and maintainer discussion of format selection, rather than pylock terminology alone. There is an important command difference: `uv add` imports project requirements and does not support pylock files at all, whereas `uv pip install` supports named pylock files. The earlier discussion nevertheless directly addresses identifying pylock content from stdin. Its closure did not implement that capability.

## Related

- astral-sh/uv#13400 — uv add fails when providing a pylock.toml via stdin (issue, closed). Closest prior discussion: piping pylock.toml through -r - produces the identical comparison-operator error at lock-version. A maintainer explicitly explains the difficulty of identifying the format without a filename. The command was uv add, which also lacks pylock file support, but the stdin format-selection limitation is shared.
- astral-sh/uv#13421 — Reject `pylock.toml` in `uv add -r` (pull request, merged). Follow-up to astral-sh/uv#13400 through astral-sh/uv#13420. Its diff only rejects pylock.toml inputs in uv add; it did not implement stdin format selection. The earlier issue's closure therefore does not establish a fixed stdin feature that has regressed.
- astral-sh/uv#13117 — Better error message when a non-PEP-751-compatible filename lock file is used (issue, closed). Reports the same lock-version comparison-operator error when a pylock file is named test.toml, demonstrating the related filename-based format-selection behavior. Its request concerns invalid filenames and diagnostics, rather than reading stdin.
- astral-sh/uv#12992 — Add `pylock.toml` to `uv pip install` and `uv pip sync` (pull request, merged). Introduced installation from named pylock.toml files, establishing the working file-based capability described in the report. Its discussion does not establish support for pylock content supplied through stdin.
- astral-sh/uv#17119 — Support remote `pylock.toml` files (pull request, merged). Fixed astral-sh/uv#17112 by adding HTTP(S) pylock inputs. The diff handles files and URLs, which retain filenames; it does not add stdin detection and is not a historical fix for this report.

- pypa/pip#7822 — Cited by the maintainer for pip’s lack of support for the `-r -` stdin convention. This is an adjacent input-handling issue, not evidence that pip supports pylock streams. Its current status and discussion were not independently checked in this update.

## Supporting evidence

- `crates/uv-requirements/src/sources.rs:39`: `RequirementsSource::from_requirements_file` maps `RequirementsInput::Stdin` to `Extensionless`. Named local and remote pylock inputs are selected using their filenames.
- `crates/uv-requirements/src/specification.rs:378`: extensionless input is read and checked for PEP 723 script metadata. Otherwise it is passed to `RequirementsTxt::parse_str`; there is no pylock detection in this branch. This source path explains the reported requirement-parser error.
- `crates/uv-pip-commands/src/pylock.rs:56`: the low-level pylock reader already has a stdin branch. That does not make the reported CLI invocation work, because requirements-source selection does not identify its stdin as pylock. The missing behavior is format selection at the CLI input path.
- `crates/uv-cli/src/lib.rs:2017`: the requirements option lists supported file formats and stdin. The report's proposed input-format flag is a requested interface change, not an existing remedy.
- In astral-sh/uv#13400, opened May 12, 2025, the reporter supplied pylock content through `uv add -r -` and received the same error. On May 13, a maintainer explicitly said there was no clear way to differentiate the input format without a filename, then opened astral-sh/uv#13420 for the separate silent acceptance of named pylock files by `uv add`.
- astral-sh/uv#13421 merged May 13, 2025. Its diff adds an error for `RequirementsSource::PylockToml` in `uv add`; it does not implement stdin support.
- astral-sh/uv#12992 merged April 21, 2025 and introduced named pylock installation. The broader support discussion in astral-sh/uv#12584 links that implementation.
- astral-sh/uv#13117 concerns the same error for a nonconforming filename. Its linked astral-sh/uv#13119 addresses filename validation in compile/export, not stdin input.
- astral-sh/uv#17119 merged December 17, 2025 and closed astral-sh/uv#17112. Its diff adds fetching over HTTP(S), with no stdin detection. Neither this change nor the other inspected historical changes establishes a regression.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Literal searches covered pylock, stdin, lock-version, and the comparison-operator error. Conceptual searches covered standard input, pipes, lockfiles, input format, format selection, detection, filenames, and extensionless requirements. PR keyword searches returned no candidates; relevant merged PRs were located through issue comments and timelines and inspected directly. Followed astral-sh/uv#13400 to astral-sh/uv#13420 and astral-sh/uv#13421, and reviewed filename-validation and remote-input fixes. Ruled out astral-sh/uv#17883, which concerns empty input after an upstream pipeline failure, and astral-sh/uv#7236, which concerns help text. No inspected fix established prior pylock stdin support.

The report was decomposed before searching into: (1) installing pylock content from stdin; (2) the exact requirements-parser error; (3) explicit input-format selection; and (4) content-based detection. Searches for observable behavior were kept separate from filename-routing searches. Broad pylock and stdin searches included both open and closed issues. Pull request searches included explicit open, closed, and merged queries; direct PR views, comments, reviews, and selected diffs supplemented their empty keyword results.

astral-sh/uv#17883 was inspected because it uses the same `uv pip install -r -` command. Its discussion concerns an upstream command failing and leaving empty input, rather than valid TOML reaching the wrong parser. astral-sh/uv#7236 requests clearer short help for supported requirement-file formats. Neither is a duplicate target for this report.

## Validation and handoff

The mechanism was checked in source; no runtime reproduction or tests were run. No repository files or GitHub state were changed. The draft response is for maintainer review and has not been posted.
