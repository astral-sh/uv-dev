# Windows: uv pip install --overrides fails when requirements file path contains spaces

Issue: astral-sh/uv#21477

Classification: duplicate

## Summary

On Windows 11 with uv 0.12.9, a quoted requirements-file path containing a space is split when passed to `uv pip install --overrides`. The reproduction creates `D:\uv test\override.txt`, but uv reports `File not found: D:\uv`; moving the override file to a space-free path succeeds. A space-free `--python` path does not change the failure, which isolates the trigger to the override-file argument.

This is a more specific reproduction of the cross-platform problem tracked in astral-sh/uv#12639. That issue demonstrates the same truncation for a quoted `--constraint` path; the CLI definitions inspected during triage used the same space-delimited CLI/environment parsing pattern for both options. A repository member also identified astral-sh/uv#21477 as a duplicate of astral-sh/uv#12639. Maintainer zanieb reports that astral-sh/uv#22290 should fix this behavior and was released in uv 0.13.0.

## Fix status and verification

In the October 16, 2026 comment on astral-sh/uv#21477, maintainer zanieb identifies astral-sh/uv#22290 as the expected fix, released in uv 0.13.0. According to the comment, the argument parser now keeps a quoted path passed to `--overrides` intact instead of splitting it on whitespace. This supersedes the earlier status of a proposed fix in astral-sh/uv-dev#158.

The supplied discussion contains no reporter confirmation of a successful retest on uv 0.13.0. The next verification step is to rerun the original Windows PowerShell reproduction on uv 0.13.0 or later and confirm that uv reads `D:\uv test\override.txt` without the truncated-path error. The release and behavior claims above are maintainer-reported; no new reproduction was performed for this handoff update.

## Classification

Duplicate of astral-sh/uv#12639. The reported behavior matches: a file path supplied as one quoted CLI argument is split at its first space and uv attempts to open only the prefix. Although the existing report uses `--constraint` and astral-sh/uv#21477 uses `--overrides`, source inspected during triage declared both arguments as lists with a space value delimiter and the same environment-variable integration. Maintainer comments on astral-sh/uv#12639 identify that shared parsing as the reason the affected command-line parser could not distinguish a path space from the environment variable's multi-file separator.

This classification is now additionally supported by a repository member's comment on astral-sh/uv#21477 identifying it as a duplicate of astral-sh/uv#12639.

The draft astral-sh/uv-dev#158 predates astral-sh/uv#21477 and explicitly addresses quoted constraint, override, and exclude paths by keeping explicit CLI values intact while splitting environment-provided lists. The issue is therefore already tracked closely enough to centralize discussion, rather than being a newly returned fixed regression.

## Related

- astral-sh/uv#12639 — Canonical duplicate discussion with the same failure on `--constraint`: `constraints file.txt` is truncated to `constraints`. Maintainer comments confirm the shared command-line/environment-variable space-delimited parsing limitation.
- astral-sh/uv#22290 — Fix identified by maintainer zanieb as released in uv 0.13.0. The maintainer states that quoted `--overrides` paths are now kept intact instead of split on whitespace; the supplied discussion does not yet include a reporter retest.
- astral-sh/uv-dev#158 — Earlier draft pull request, open at initial triage, that explicitly covers constraint, override, and exclude paths. It proposes keeping explicit CLI paths intact and splitting only environment-provided lists, directly addressing the shared problem in astral-sh/uv#12639. Retained as historical implementation context; the maintainer points to astral-sh/uv#22290 for the released fix.

## Supporting evidence

- Literal searches covered `--overrides`, `File not found`, `override.txt`, Windows, quoted paths, and paths containing spaces across open and closed issues and pull requests.
- Conceptual searches covered whitespace and argument splitting, requirements-file and constraint-file paths, CLI versus environment parsing, and analogous space-delimited options.
- Fix-oriented review covered closed issues and merged pull requests as well as open drafts. astral-sh/uv#15806 and merged astral-sh/uv#15815 confirm an analogous Clap delimiter limitation for `--env-file`, but they concern a different option and escaping implementation rather than directly tracking requirement-file arguments. astral-sh/uv#17227 was inspected and ruled out because it concerns the `-` stdin sentinel after a separate regression, not whitespace in a path.
