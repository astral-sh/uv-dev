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

Current source is consistent with the observation: `run.rs` checks whether the direct target or
`--from` value has a `.py`/`.pyw` path extension before calling `ToolRequest::parse`, then passes the
complete URL to `PackageName::from_str` in this case.

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

Existing integration coverage is adjacent but does not cover the collision:

- `crates/uv/tests/tool/tool_run.rs::tool_run_git` verifies successful direct and `--from` Git
  requirements whose repository basename does not end in `.py`.
- `crates/uv/tests/tool/tool_run.rs::tool_run_with_existing_py_script`,
  `tool_run_with_nonexistent_py_script`, and `tool_run_with_from_script` verify the intended errors
  for actual or apparent Python script paths.
- No existing test in that file combines a Git requirement with a `.py` repository basename.

## Draft response

Thanks for the report. Git requirements are supported in both forms shown, but the current `.py`
script-path check runs before the tool requirement is parsed. As a result, a Git URL whose
repository name ends in `.py` is incorrectly treated as a script path and then validated as a
package name.

This is distinct from astral-sh/uv#10784, which intentionally handles actual `.py` script paths.
The next step is to narrow that check to avoid intercepting Git requirements and add coverage for
both the direct and `--from` forms.

## Classification

This is a reproducible bug. Git requirements are an established `uvx` capability, and current
integration tests cover successful direct and `--from` invocations. The extension-only pre-parse
guard incorrectly intercepts a valid Git requirement. No open issue or pull request already tracks
this behavior, so astral-sh/uv#21141 is not a duplicate.

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

Current integration tests in `crates/uv/tests/tool/tool_run.rs` cover successful Git requirements in
both reported invocation forms and separately cover the intended errors for local `.py` and `.pyw`
script names. They do not cover a Git requirement whose final path component has one of those
extensions. The local reproduction above supplies direct behavioral evidence in addition to that
source and test inspection.
