# Metadata hook observation

Run against an explicitly selected uv executable:

```sh
python scripts/research/metadata_hooks.py --uv /path/to/uv --work-dir work --check
```

The dependency-free synthetic backend logs module imports and PEP 517 hooks. Six fixtures exercise
static project metadata, static PEP 643 sdist metadata, dynamic metadata preparation, and
wheel-build fallback. Each runs `uv pip compile` with a fresh cache and again with that cache
populated. The local dependency wheel makes the dependency selected from each metadata path
observable. No registry access is needed; the selected Python interpreter is reported alongside the
uv version.

JSON output includes hook calls, resolved fixture dependencies, return codes, diagnostics, and
elapsed time. `--check` requires the expected resolved dependencies, zero backend execution for the
static cases, and the appropriate dynamic fallback. These are small behavioral fixtures, not
representative performance measurements or a claim about static metadata coverage across PyPI.
Resolving dynamic metadata runs the bundled backend with ordinary host process permissions; it is
not sandboxed.

The experiment complements real-project metadata timing by exposing _which code executes_.
Extensions should preserve [PEP 643](https://peps.python.org/pep-0643/) dynamic-field semantics.
[PyFEX](https://arxiv.org/abs/2606.02196) motivates making execution visible, while
[No Snake Oil](https://arxiv.org/abs/2607.21888) motivates checking source/artifact agreement.
Neither establishes that static metadata or a matching dependency list makes a package benign.

The harness also invokes `uv build` separately for every fixture and reads the resulting wheel's
`Requires-Dist` headers. Two deliberately inconsistent backends change the wheel dependency to
`metadata-dep==2.0` while static project metadata or the metadata hook advertises
`metadata-dep==1.0`. `--check` requires these negative controls to disagree and the four consistent
controls to agree. Build hooks and timings are reported separately from resolution, so the extra
build does not contaminate the cold/warm metadata observations. These intentionally invalid backends
demonstrate detection by the experiment, not a newly claimed uv defect or a complete semantic
metadata equivalence check.

Failed commands or expectation checks retain accumulated observations and binary provenance in the
JSON report, then exit nonzero. Wheel-fallback fixtures remove the optional hook by deleting its
binding after defining the backend.
