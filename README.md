# unable to `uv pip compile` with `--python-version=3.14t`

Issue: astral-sh/uv#21149

Classification: duplicate

## Summary

The report shows that `uv pip compile --python=3.14t pyproject.toml` succeeds, while the otherwise similar `--python-version=3.14t` form fails during CLI parsing. The immediate difference is source-confirmed: `--python` is parsed as a `PythonRequest`, whose supported request formats include free-threaded variants such as `3.14t`, whereas `--python-version` is parsed as a `PythonVersion`, which accepts a PEP 440-style version but not the `t` variant suffix.

The underlying duplication between `PythonVersion` and the interpreter request representation is already tracked by astral-sh/uv#3708. The `pip compile` implementation also contains a TODO to consolidate `VersionRequest` and `PythonVersion` at the point where a target version is converted into an interpreter request. This report is therefore a concrete user-facing reproduction of that existing internal split.

## Draft response

Thanks — this is the user-facing consequence of the separate `PythonVersion` and `VersionRequest` representations already tracked in astral-sh/uv#3708. `--python` accepts interpreter-request variants such as `3.14t`, while `--python-version` currently goes through `PythonVersion`, so it rejects the `t` suffix before resolution.

Let’s centralize the representation work in astral-sh/uv#3708; I’m marking this as a duplicate. For now, `uv pip compile --python=3.14t pyproject.toml` is the available way to select the free-threaded interpreter, as shown in your reproduction.

## Classification

Duplicate of astral-sh/uv#3708. That open issue explicitly tracks combining `PythonVersion` and `VersionRequest`, which is the exact representation split identified here. The new issue adds the useful `uv pip compile` and `3.14t` reproduction, but it does not establish a distinct underlying problem or a regression of a previously merged fix.

The neighboring suffix request in astral-sh/uv#12950 is not the canonical duplicate: despite its title mentioning `python-version`, it concerns `3.14-dev` and `3.14t-dev` interpreter requests supplied through `UV_PYTHON`/setup-uv. Likewise, astral-sh/uv#16434 concerns discovery rejecting a Python 3.13 free-threaded interpreter against `requires-python`; it uses a different command, trigger, and matching path. The fixes associated with astral-sh/uv#16253 changed variant display and GIL-enabled interpreter selection, not `pip compile --python-version` parsing, so this is not evidence of a regression.

## Related

- astral-sh/uv#3708 — **Combine `PythonVersion` and `VersionRequest`** (open issue). This is the closest and canonical match because it tracks the same duplicated representations responsible for the reported inconsistency.
- astral-sh/uv#3266 — **Rewrite Python interpreter discovery** (merged pull request). It introduced the interpreter-request architecture and listed combining `PythonVersion` and `VersionRequest` as future work; it provides the provenance for astral-sh/uv#3708 but did not implement the consolidation.
- astral-sh/uv#12950 — **Support 3.14-dev and 3.14t-dev in `python-version`** (open issue). This is adjacent suffix-syntax work, but it concerns development-release interpreter requests passed through `UV_PYTHON`, not the `uv pip compile --python-version` target-version parser.

## Search and supporting evidence

The report was decomposed into the `uv pip compile` command, the `--python-version` parse failure, the successful `--python` comparison, the `3.14t` free-threaded selector, and the `PythonVersion`/`PythonRequest` type distinction. Searches covered exact `3.14t` and error wording; `pip compile` with `python-version`; `PythonVersion` and `PythonRequest`; free-threaded/freethreaded selector and resolver terminology; open and closed issues; and open, closed, and merged pull requests. Version-specific historical fixes were inspected as well.

Repository source confirms that `PipCompileArgs::python_version` is an `Option<PythonVersion>`, `PythonVersion::from_str` delegates to the PEP 440 version parser, and `PythonRequest`/`VersionRequest` explicitly test and document the `3.13t` form. The `pip compile` implementation carries the comment `TODO(zanieb): We should consolidate VersionRequest and PythonVersion` at the conversion between these representations. No open recent pull request references astral-sh/uv#21149 or attempts to change this behavior.
