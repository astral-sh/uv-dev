# unable to `uv pip compile` with `--python-version=3.14t`

Issue: astral-sh/uv#21149

Classification: question

## Summary

The report shows that `uv pip compile --python=3.14t pyproject.toml` succeeds, while the otherwise similar `--python-version=3.14t` form fails during CLI parsing. The immediate difference is source-confirmed: `--python` is parsed as a `PythonRequest`, whose supported request formats include free-threaded variants such as `3.14t`, whereas `--python-version` is parsed as a `PythonVersion`, which accepts a PEP 440-style version but not the `t` variant suffix.

An initial maintainer clarification states that this distinction is intentional: `3.14t` denotes a build variant rather than a Python version. `--python-version` specifies the version used for resolution, while `--python` selects a specific interpreter and therefore accepts build-variant selectors.

The reporter's follow-up adds an important unresolved consequence: free-threaded Python uses a distinct ABI tag, so the build variant can affect which wheels and lock entries are selected. The successful `uv pip compile --python=3.14t pyproject.toml` form preserves that ABI information by selecting a free-threaded interpreter, but callers that only construct `--python-version` cannot express the same target variant. The reporter has asked whether that limitation is also intentional; there is no maintainer answer to that narrower question yet.

## Reproduction and impact

The follow-up points to pypa/hatch#2395, where Hatch passes its configured Python value to `uv pip compile --python-version`. That pull request proposes stripping `t` from `3.14t` so uv accepts the argument. The reporter notes that this normalization is not semantically neutral because it discards the free-threaded ABI selection.

A concrete reported comparison compiles a requirements file containing packages with free-threaded wheels—including `cryptography`, `cffi`, `pyyaml`, `numpy`, `pydantic-core`, `pandas`, `aiohttp`, `pillow`, `scipy`, and `lxml`—to `pylock.toml` once with `--python=3.14` and once with `--python=3.14t`, then diffs the outputs. This is a useful reproduction for determining which artifacts or lock entries differ. It has not been independently executed in this handoff.

Repository source supports the underlying mechanism: when no `--python-version` override is present, `resolution_tags` uses the selected interpreter's tags, including whether the GIL is disabled. When `--python-version` is supplied, the target version is overridden but the GIL-disabled/debug properties still come from the build interpreter; the option has no syntax for independently selecting a target build variant.

## Classification

Question, pending further maintainer clarification. A repository maintainer confirmed the semantic reason for the parsing difference: `--python-version` accepts a version, while `3.14t` includes a free-threaded build-variant selector and belongs with the interpreter-selecting `--python` option. The follow-up does not show that `--python=3.14t` resolves incorrectly, but it does establish that the variant affects dependency resolution and raises a narrower unanswered question: whether uv should support expressing a target ABI variant without selecting a specific interpreter.

The earlier duplicate assessment against astral-sh/uv#3708 is superseded by this clarification. That issue tracks an internal consolidation of representations, but it does not establish that build variants should become valid `--python-version` values. The neighboring suffix request in astral-sh/uv#12950 concerns `3.14-dev` and `3.14t-dev` interpreter requests supplied through `UV_PYTHON`/setup-uv. Likewise, astral-sh/uv#16434 concerns discovery rejecting a Python 3.13 free-threaded interpreter against `requires-python`; it uses a different command, trigger, and matching path. The fixes associated with astral-sh/uv#16253 changed variant display and GIL-enabled interpreter selection, not `pip compile --python-version` parsing, so this is not evidence of a regression.

## Related

- astral-sh/uv#3708 — **Combine `PythonVersion` and `VersionRequest`** (open issue). This is related internal cleanup, but the maintainer's semantic distinction between versions and build variants means it is not a duplicate of the reported behavior.
- astral-sh/uv#3266 — **Rewrite Python interpreter discovery** (merged pull request). It introduced the interpreter-request architecture and listed combining `PythonVersion` and `VersionRequest` as future work, but did not state that build variants should be accepted as target versions.
- astral-sh/uv#12950 — **Support 3.14-dev and 3.14t-dev in `python-version`** (open issue). This is adjacent suffix-syntax work, but it concerns development-release interpreter requests passed through `UV_PYTHON`, not the `uv pip compile --python-version` target-version parser.
- pypa/hatch#2395 — **Strip the build variant when passing a Python version to uv** (open pull request). Hatch currently forwards configured Python values to uv's `--python-version`; the proposed normalization from `3.14t` to `3.14` avoids the parse error but may change resolution by discarding the free-threaded ABI target.

## Search and supporting evidence

The report was decomposed into the `uv pip compile` command, the `--python-version` parse failure, the successful `--python` comparison, the `3.14t` free-threaded selector, and the `PythonVersion`/`PythonRequest` type distinction. Searches covered exact `3.14t` and error wording; `pip compile` with `python-version`; `PythonVersion` and `PythonRequest`; free-threaded/freethreaded selector and resolver terminology; open and closed issues; and open, closed, and merged pull requests. Version-specific historical fixes were inspected as well.

Repository source confirms that `PipCompileArgs::python_version` is an `Option<PythonVersion>`, `PythonVersion::from_str` delegates to the PEP 440 version parser, and `PythonRequest`/`VersionRequest` explicitly test and document the `3.13t` form. The `pip compile` implementation carries the comment `TODO(zanieb): We should consolidate VersionRequest and PythonVersion` at the conversion between these representations. No open recent pull request references astral-sh/uv#21149 or attempts to change this behavior.

The maintainer comment resolves why the two CLI options intentionally accept different categories of input, and the internal consolidation TODO should not by itself be read as evidence that they must accept identical values. The follow-up nevertheless shows that build-variant information is resolution-relevant, leaving open whether a separate target-variant mechanism is needed.
