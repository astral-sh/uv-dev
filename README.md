# `uv check --script` ignores `exclude-newer-package` exemptions when selecting `ty`

Issue: astral-sh/uv#21211

Classification: bug

## Summary

The reported behavior is reproducible with the installed uv 0.12.5. An isolated PEP 723 script
with a global seven-day `exclude-newer` cutoff and `exclude-newer-package = { ty = false }` selected
ty 0.0.70. The same script without any cutoff selected ty 0.0.73, while a global-cutoff-only
control selected ty 0.0.70. This shows that the package-specific exemption does not affect
standalone ty selection in `uv check --script`.

The script's inline metadata is sufficient to reproduce the behavior; the reported project-level
`pyproject.toml` is not required. The behavior was observed on Linux as well as reported on macOS,
so it is not limited to the reporter's platform.

## Reproduction

Outcome: **reproducible**.

Environment:

- uv 0.12.5 (`x86_64-unknown-linux-gnu`)
- Python 3.12.3
- Linux x86_64
- Reproduced on 2026-08-19 with all files, caches, tool directories, and XDG state under fresh
  `$RUNNER_TEMP` directories

Minimal `script.py`:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = []
#
# [tool.uv]
# exclude-newer = "P7D"
# exclude-newer-package = { ty = false }
# ///

value: int = 1
```

Commands and observed results:

```console
$ uvx --no-config ty@latest --version
ty 0.0.73

$ uv check --script script.py --show-version --preview-features=check-command
Using ty 0.0.70
All checks passed!
```

Two isolated controls disambiguate the result. Removing only
`exclude-newer-package = { ty = false }` still selected ty 0.0.70. Removing the entire `[tool.uv]`
table selected ty 0.0.73. The same three-way result was observed with `--ty-version latest`:
exemption and global-only both selected 0.0.70, while no cutoff selected 0.0.73.

There is no existing integration test for the package-specific override on standalone ty
selection. `crates/uv/tests/project/check.rs::check_script` covers basic PEP 723 checking, and
`crates/uv/tests/project/check.rs::check_script_ignores_transitive_ty_for_tool_selection` covers a
global `--exclude-newer` cutoff, but neither supplies `exclude-newer-package`. General resolver
semantics are covered by
`crates/uv/tests/lock/lock.rs::lock_exclude_newer_package_disable`, which asserts that
`idna=false` exempts `idna` from the global cutoff while a non-exempt dependency remains filtered.

The observed behavior is consistent with the current command boundary:
`crates/uv/src/commands/project/check.rs` extracts only
`settings.resolver.exclude_newer.global` before calling the standalone ty resolver. The behavioral
controls, rather than source inspection alone, establish the reproduction.

## Draft response

Thanks for the clear report. This is reproducible with uv 0.12.5 on Linux as well as the reported
macOS platform. In an isolated script, the `ty = false` case selected ty 0.0.70, the same as a
global-cutoff-only control, while removing the cutoff selected ty 0.0.73. The inline script metadata
alone is enough to reproduce the issue.

The general `false` exemption is covered for dependency resolution, but there is no equivalent
integration coverage for standalone ty selection in `uv check --script`. The current command path
passes only the global timestamp to that resolver, consistent with the observed result.

## Classification

`bug` is appropriate. A package-specific value of `false` is documented and tested as an exemption
from the global `exclude-newer` cutoff, but the targeted reproduction shows that standalone ty
selection behaves identically with the exemption present and absent.

This is not a duplicate based on the existing related-issue search. astral-sh/uv#16854 established
the general exemption behavior, while astral-sh/uv#19605 and astral-sh/uv#19989 concern the command
path and locked-tool selection rather than a prior fix for this omission.

## Related

- astral-sh/uv#16854 — **Allow disabling `exclude-newer` per package** (merged pull request). It
  implemented the `PACKAGE=false` opt-out and establishes that a disabled package-specific value
  overrides the global cutoff.
- astral-sh/uv#19605 — **Add `uv check` to run `ty` from uv** (merged pull request). Its diff
  introduced standalone ty selection with the global-only cutoff handoff that remains in the
  current source.
- astral-sh/uv#19989 — **Use locked dependency selection for `uv check --script`** (merged pull
  request). This handles a directly declared and locked ty dependency; the minimal reproduction
  declares no dependencies and reaches standalone tool selection instead.

## Search evidence

Existing searches covered open and closed issues and open, closed, and merged pull requests using
literal and conceptual variants of `exclude-newer-package`, `exclude-newer`, `uv check --script`,
`ty`, package-specific cutoffs, configuration propagation, and standalone tool selection. No exact
prior report or fix was found.

The strongest apparent match, astral-sh/uv#19239, was ruled out: that report involved non-exempt
transitive dependencies constraining `uv pip compile`, not a discarded exemption during standalone
tool selection.
