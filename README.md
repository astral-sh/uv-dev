# Windows: uv pip install --overrides fails when requirements file path contains spaces

Issue: astral-sh/uv#21477

Classification: duplicate

## Summary

On Windows 11 with uv 0.12.9, a quoted requirements-file path containing a space is split when passed to `uv pip install --overrides`. The reproduction creates `D:\uv test\override.txt`, but uv reports `File not found: D:\uv`; moving the override file to a space-free path succeeds. A space-free `--python` path does not change the failure, which isolates the trigger to the override-file argument.

This is a more specific reproduction of the open cross-platform problem in astral-sh/uv#12639. That issue demonstrates the same truncation for a quoted `--constraint` path, and the two options use the same space-delimited CLI/environment parsing pattern. A pre-existing draft fix covering constraint, override, and exclude paths is available in astral-sh/uv-dev#158.

## Draft response

Thanks for the clear reproduction. This is the same underlying file-path parsing problem tracked in astral-sh/uv#12639: `--constraint` and `--overrides` currently use the same space-delimited parsing as their environment-variable forms, so quoting a CLI path does not prevent it from being split. A draft fix covering constraint, override, and exclude paths is in astral-sh/uv-dev#158. Let’s centralize the discussion in astral-sh/uv#12639.

## Classification

Duplicate of astral-sh/uv#12639. The observable behavior matches: a file path supplied as one quoted CLI argument is split at its first space and uv attempts to open only the prefix. Although the existing report uses `--constraint` and astral-sh/uv#21477 uses `--overrides`, source evidence shows both arguments are declared as lists with a space value delimiter and the same environment-variable integration. Maintainer comments on astral-sh/uv#12639 identify that shared parsing as the reason the command line cannot currently distinguish a path space from the environment variable's multi-file separator.

The draft astral-sh/uv-dev#158 predates astral-sh/uv#21477 and explicitly addresses quoted constraint, override, and exclude paths by keeping explicit CLI values intact while splitting environment-provided lists. The issue is therefore already tracked closely enough to centralize discussion, rather than being a newly returned fixed regression.

## Related

- astral-sh/uv#12639 — Open issue with the same failure on `--constraint`: `constraints file.txt` is truncated to `constraints`. Maintainer comments confirm the shared command-line/environment-variable space-delimited parsing limitation.
- astral-sh/uv-dev#158 — Open draft pull request that explicitly covers constraint, override, and exclude paths. It proposes preserving explicit CLI paths and splitting only environment-provided lists, directly addressing the shared problem in astral-sh/uv#12639.

## Supporting evidence

- Literal searches covered `--overrides`, `File not found`, `override.txt`, Windows, quoted paths, and paths containing spaces across open and closed issues and pull requests.
- Conceptual searches covered whitespace and argument splitting, requirements-file and constraint-file paths, CLI versus environment parsing, and analogous space-delimited options.
- Fix-oriented review covered closed issues and merged pull requests as well as open drafts. astral-sh/uv#15806 and merged astral-sh/uv#15815 confirm an analogous Clap delimiter limitation for `--env-file`, but they concern a different option and escaping implementation rather than directly tracking requirement-file arguments. astral-sh/uv#17227 was inspected and ruled out because it concerns the `-` stdin sentinel after a separate regression, not whitespace in a path.
