# `uv run` cannot properly resolve a dependency on `onnxruntime` declared in `pyproject.toml`

Issue: astral-sh/uv#21820

Classification: duplicate

## Summary

The report shows `uv run` performing a universal project resolution for `requires-python = ">=3.10"`. The resolver forks newer `onnxruntime` releases at Python 3.11, but selects `onnxruntime==1.24.3` for the Python 3.10 fork. That release declares Python 3.10 as supported yet provides only cp311 through cp314 wheels and no source distribution, so synchronization fails with the reported no-compatible-distribution error. By contrast, the environment-specific `uv pip install` selects `onnxruntime==1.23.2`, which has a cp310 wheel.

This is the behavior tracked generally by astral-sh/uv#9711 and in the same package-and-Python form by astral-sh/uv#9425. Project commands resolve for the supported range rather than privileging the active interpreter; a package version without an sdist can currently be accepted when at least one wheel is compatible with the range even though a particular supported Python version has no artifact.

## Draft response

Thanks for the detailed reproduction. This is another instance of the incomplete universal-resolution behavior tracked in astral-sh/uv#9711, and the same Python-specific onnxruntime case already reported in astral-sh/uv#9425. In project mode, uv resolves across the project’s supported Python range rather than privileging the active interpreter. onnxruntime 1.24.3 declares Python 3.10 as supported, but publishes neither a cp310 wheel nor a source distribution, so that metadata is not enough to make the selected version installable on CPython 3.10.

As a workaround, add:

```toml
[tool.uv]
required-environments = ["python_version == '3.10'"]
```

This makes the lock require a compatible Python 3.10 wheel. Alternatively, constrain onnxruntime to `<1.24.3` for Python 3.10. We’ll keep the general behavior centralized in astral-sh/uv#9711.

## Classification

This is a duplicate because two open issues already track the same underlying problem. astral-sh/uv#9711 is the canonical general tracker, and astral-sh/uv#9425 is an onnxruntime-specific report in which project resolution selects a release whose declared Python support is broader than its published wheel set. The new report changes the versions from Python 3.9 and onnxruntime 1.20.x to Python 3.10 and onnxruntime 1.24.3, but the triggering condition, project-mode behavior, actual failure, and requested fallback are the same.

The presence of a `Requires-Python: >=3.10` entry does not distinguish this case: that metadata admits Python 3.10 while the available artifacts begin at cp311. Merged astral-sh/uv#9827 added automatic forks when a package version raises its `Requires-Python` lower bound, which explains the newer-version forks in the log, but it does not use wheel implementation tags to subdivide a Python range that the metadata says is supported. This is therefore not a regression of that fix.

The no-`pyproject.toml` comparison also demonstrates a different resolver mode. `uv pip install` resolves for the selected environment, whereas project commands create a universal lock. In addition, the subsequent `uv run` output in that comparison reports Python 3.13.15 rather than the CPython 3.10 environment created immediately before it; the `uv pip install` result is the relevant evidence for the environment-specific resolution difference.

## Related

- astral-sh/uv#9711 — Open canonical tracker for packages without source distributions receiving an incomplete universal resolution. Its description states that a package version is accepted for all Python versions and platforms once at least one compatible wheel exists, matching the selection of onnxruntime 1.24.3 for the Python 3.10 fork.
- astral-sh/uv#9425 — Open onnxruntime-specific report with the same project-mode failure and environment-specific fallback. There, onnxruntime metadata also admitted a Python version for which the release published no matching wheel; maintainers suggested `required-environments` as the workaround.
- astral-sh/uv#11274 — Closed near-identical onnxruntime project reproduction. Maintainer comments explain that universal resolution does not privilege the running interpreter and relies on package metadata rather than requiring an artifact for every possible environment.
- astral-sh/uv#9827 — Merged related resolver change that forks versions when their `Requires-Python` bounds narrow the project range. It does not cover releases whose declared range includes a Python version missing from the wheel set.

## Supporting evidence

Literal searches covered `onnxruntime`, `onnxruntime==1.24.3`, `cp310`, the exact “doesn't have a source distribution or wheel for the current platform” error, and the implementation-tag hint. Conceptual searches covered universal and project resolution, wheel availability, packages without sdists, Python-version forks, `Requires-Python`, required environments, lockfiles, and the difference between `uv pip` and `uv run`/`uv add`/`uv sync`. Fix-oriented searches included closed issues and merged pull requests.

astral-sh/uv#5182 and merged astral-sh/uv#10046 were inspected but are not canonical for this report: their shipped behavior is narrowly concerned with choosing non-local PyTorch versions when local versions lack platform support. Closed astral-sh/uv#8492 and merged astral-sh/uv#9827 concern releases with elevated `Requires-Python` lower bounds, not a missing wheel within the range declared by the metadata.
