# Finite universal objective oracle

```sh
python scripts/research/universal_objectives.py --uv /absolute/path/to/uv --work-dir "$PWD/work"
```

The command runs both universal fork strategies over six deterministic local wheels, then projects
each universal result through uv for Python 3.11 and 3.12. An independent exhaustive enumerator
checks the projected assignments against the fixture's dependency rules and computes the exact
Pareto frontier for three explicitly defined costs, all minimized:

- The total number of distinct versions across environments, summed over packages.
- The version-rank distance from each environment's newest eligible versions, summed over pins.
- The number of environment/package pins changed from an explicit all-version-1 baseline.

The JSON includes the fixture fingerprint, raw universal output, projected choices, objective
vectors, and distance from each independent minimum. Independent minima need not be jointly
attainable. A nonzero gap is not a correctness failure or a claim that uv promises global
optimality.

The experiment first generates the reference assignment using `uv pip compile --resolution lowest`.
For each fork strategy it then measures three modes: a cold resolution, a rerun whose
`--output-file` contains that previous output, and the same seeded rerun with `--upgrade`. The
`measurements` object contains cold results; `preference_measurements` contains the two seeded
modes. Their churn metric is relative to the actual previous output supplied to uv. The cold run
only uses that output as a counterfactual reference. This exercises requirements-file preferences
rather than `uv.lock` reuse.

Projection uses `--no-deps` so uv cannot repair a missing transitive pin. Negative controls delete a
transitive pin and replace one with a dependency-inconsistent version; the finite closure and
dependency checks must reject both. Their outcomes appear in `projection_controls`.

This is a deliberately finite objective-quality experiment, following the separation between
validity and optimization in [PacSolve](https://arxiv.org/html/2203.13737v4). The two Python cells
cover every `Requires-Python` boundary in this fixture's `>=3.11,<3.13` range. It does not model
extras, prereleases, indexes, URLs, or platform-specific artifacts. All candidates are local pure
wheels; network access, source builds, and persistent caches are disabled. It does not replace the
Packse scenario correctness oracle.
