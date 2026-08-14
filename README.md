# uvx: git url is rejected as a package name when the repository name ends in .py

Issue: astral-sh/uv#21141

Classification: bug

## Summary

The behavior in astral-sh/uv#21141 is reproducible with uv 0.12.4. Both supported forms of running a
tool from a Git requirement reject a repository URL whose basename ends in `.py`:

```console
uvx git+https://github.com/uPesy/easyeda2kicad.py
uvx --from git+https://github.com/uPesy/easyeda2kicad.py easyeda2kicad
```

Both commands reject the entire URL as an invalid package or extra name before any Git operation.
A local, trusted Git fixture reproduced the same result in both forms, while an otherwise identical
repository with a basename that does not end in `.py` ran successfully.

The pre-fix source is consistent with the observation: `run.rs` checked whether the direct target
or `--from` value had a `.py`/`.pyw` path extension before calling `ToolRequest::parse`, then passed
the complete URL to `PackageName::from_str` in this case.

No existing issue or pull request was found that already tracks this `.py` Git-URL collision. The
closest history is the intentional handling of actual Python script paths added for
astral-sh/uv#10784 by astral-sh/uv#11623.

## Reproduction

Outcome: **reproducible**.

The reproduction used the installed `uvx 0.12.4 (x86_64-unknown-linux-gnu)` on Linux x86_64 with
Python 3.12.3 available. The reporter used the same uv version on OpenSUSE Tumbleweed x86_64 with
Python 3.12.13. The failure occurs before Python selection or Git access.

A temporary local Git repository named `fixturetool.py` contained this minimal package metadata and
a `fixturetool` module whose `main` function prints `fixture tool ran`:

```toml
[build-system]
requires = ["setuptools"]
build-backend = "setuptools.build_meta"

[project]
name = "fixturetool"
version = "0.1.0"
requires-python = ">=3.8"

[project.scripts]
fixturetool = "fixturetool:main"

[tool.setuptools]
py-modules = ["fixturetool"]
```

With all caches and tool directories isolated under the temporary directory, both reconstructed
forms failed:

```console
$ uvx git+file:///<tmp>/fixturetool.py
error: Not a valid package or extra name: "git+file:///<tmp>/fixturetool.py". Names must start and end with a letter or digit and may only contain -, _, ., and alphanumeric characters.
$ echo $?
2

$ uvx --from git+file:///<tmp>/fixturetool.py fixturetool
error: Not a valid package or extra name: "git+file:///<tmp>/fixturetool.py". Names must start and end with a letter or digit and may only contain -, _, ., and alphanumeric characters.
$ echo $?
2
```

Copying the identical Git repository to a directory named `fixturetool` provided a control. Both
`uvx git+file:///<tmp>/fixturetool` and
`uvx --from git+file:///<tmp>/fixturetool fixturetool` built the package, printed
`fixture tool ran`, and exited 0.

Integration coverage relevant to the collision is:

- `crates/uv/tests/tool/tool_run.rs::tool_run_git` verifies successful direct and `--from` Git
  requirements whose repository basename does not end in `.py`.
- `crates/uv/tests/tool/tool_run.rs::tool_run_with_existing_py_script`,
  `tool_run_with_nonexistent_py_script`, and `tool_run_with_from_script` verify the intended errors
  for actual or apparent Python script paths.
- `crates/uv/tests/tool/tool_run.rs::tool_run_git_repository_ending_in_py`, added by the parent
  regression pull request after triage, combines those cases and is updated by the fix below.

## Fix

Outcome: **fixed**.

The script-path guard in `crates/uv/src/commands/tool/run.rs` now recognizes named and unnamed Git
requirements before applying the `.py`/`.pyw` path check. Git URLs whose repository basename ends
in `.py` therefore continue through normal tool requirement parsing, while actual existing,
missing, and `--from` Python script paths retain their existing diagnostics.

The parent regression in `crates/uv/tests/tool/tool_run.rs` was changed from snapshotting the
invalid-name error to running the temporary Git package successfully in both reported invocation
forms. Inspection also demonstrated the same false positive for the supported named PEP 508 form,
so the same regression now verifies `fixturetool @ git+file://.../fixturetool.py` as a distinct
configuration of the confirmed cause.

Focused validation succeeded:

- `cargo test --package uv --test tool tool_run_git_repository_ending_in_py`
- `cargo test --package uv --test tool tool_run_with_existing_py_script`
- `cargo test --package uv --test tool tool_run_with_nonexistent_py_script`
- `cargo test --package uv --test tool tool_run_with_from_script`
- `cargo +stable clippy --package uv --test tool -- -D warnings`
- `cargo +stable fmt --all`

## Draft response

Thanks for the report. Git requirements are supported in both forms shown, but the `.py`
script-path check ran before the tool requirement was parsed. As a result, a Git URL whose
repository name ended in `.py` was incorrectly treated as a script path and then validated as a
package name.

This is distinct from astral-sh/uv#10784, which intentionally handles actual `.py` script paths.
The check now excludes Git requirements, with coverage for the direct, `--from`, and named PEP 508
forms while preserving the diagnostics for actual script paths.

## Classification

This is a reproducible bug. Git requirements are an established `uvx` capability, and integration
tests cover successful direct and `--from` invocations. The pre-fix extension-only guard incorrectly
intercepted a valid Git requirement. No open issue or pull request already tracks this behavior, so
astral-sh/uv#21141 is not a duplicate.

## Related

- astral-sh/uv#10784 — “Hint use of `uv run` if `uvx` is given a script path” (closed). This is the
  canonical discussion for intentionally intercepting actual `.py` script paths. The new issue is
  the unintended application of that behavior to a supported Git requirement.
- astral-sh/uv#11623 — “Return error when running uvx with a .py script” (merged). This implemented
  the extension-only early checks for the direct target and `--from`; its diff and current source
  confirm the mechanism behind the false positive. It did not address Git URLs ending in `.py`.
- astral-sh/uv#8199 — “feature: uvx for github repos” (closed). This establishes the adjacent
  expected behavior: Git repositories can be used through `--from`, and direct invocation is also
  supported when the executable can be inferred. It does not cover the `.py` suffix collision.

## Search and supporting evidence

Searches covered the exact error text and sample repository, direct `uvx` Git URLs, `uvx --from`
Git requirements, repository basenames ending in `.py`, script-path handling, PEP 508/tool request
parsing, and open, closed, and merged fix history. The strongest candidates and their referenced
threads were inspected.

The exact-error candidate astral-sh/uv#6296 and merged fix astral-sh/uv#11337 were ruled out: they
addressed general extras and PEP 508 requirement support before uv 0.12.4, not the later
extension-based script guard. Git-related reports astral-sh/uv#12713, astral-sh/uv#15882, and
astral-sh/uv#16303 were also ruled out because their failures occur later during Git parsing,
authentication, or fetching, whereas this report fails during local pre-parse validation.

Integration tests in `crates/uv/tests/tool/tool_run.rs` cover successful Git requirements in both
reported invocation forms and separately cover the intended errors for local `.py` and `.pyw`
script names. The updated parent regression now also covers a Git requirement whose final path
component has a `.py` extension. The local reproduction above supplies direct behavioral evidence
in addition to that source and test inspection.

Pull request: https://github.com/astral-sh/uv-dev/pull/751
