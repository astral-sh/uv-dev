# `fork-strategy` has no effect on the forks that come from `[tool.uv] environments`

Issue: astral-sh/uv#20999

Classification: bug

## Summary

astral-sh/uv#20999 reports that the order of entries in `tool.uv.environments`
determines which Python fork is resolved first and can override the intended
`tool.uv.fork-strategy` behavior. The exact NumPy fixture is reproducible on
the currently installed uv 0.12.3; the reporter observed it on uv 0.11.23.

With the default `requires-python` strategy, listing Python 3.11 first causes
NumPy 2.4.6 to be selected for both Python 3.11 and 3.12. Reversing the entries
selects NumPy 2.4.6 for Python 3.11 and 2.5.1 for Python 3.12, which is also the
result when `environments` is omitted. With `fork-strategy = "fewest"`, the
same list-order dependence is observable in reverse: Python 3.11 first yields
one NumPy version, while Python 3.12 first yields two.

## Reproduction

Outcome: **reproducible**.

Environment used:

- Linux x86_64, kernel 6.17.0-1020-azure
- uv 0.12.3 (`x86_64-unknown-linux-gnu`), installed executable on `PATH`
- Python 3.12.3
- Fresh project and uv cache under `/tmp`

Minimal `pyproject.toml`:

```toml
[project]
name = "example"
version = "0.1.0"
requires-python = ">=3.11,<3.13"
dependencies = ["numpy"]

[tool.uv]
exclude-newer = "2026-08-01T00:00:00Z"
environments = ["python_version == '3.11'", "python_version == '3.12'"]
```

Command:

```console
uv lock -v --cache-dir /tmp/uv-issue-20999/cache
```

The verbose output for the fixture above showed:

```text
Solving split (markers: python_full_version == '3.11.*')
Selecting: numpy==2.4.6 [compatible]
Solving split (markers: python_full_version == '3.12.*')
Selecting: numpy==2.4.6 [preference]
```

Fresh-directory variants produced this matrix:

| Strategy | Environment order | Locked NumPy result |
| --- | --- | --- |
| `requires-python` (default) | 3.11, 3.12 | 2.4.6 for both forks |
| `requires-python` (default) | 3.12, 3.11 | 2.5.1 on 3.12; 2.4.6 on 3.11 |
| `fewest` | 3.11, 3.12 | 2.4.6 for both forks |
| `fewest` | 3.12, 3.11 | 2.5.1 on 3.12; 2.4.6 on 3.11 |
| `requires-python` (default) | `environments` omitted | 2.5.1 on 3.12; 2.4.6 on 3.11 |

This directly confirms the reported order dependence. The lockfiles also
confirmed that the one-version cases contained only NumPy 2.4.6, while the
two-version cases assigned NumPy 2.4.6 below Python 3.12 and NumPy 2.5.1 at
Python 3.12 and above.

There is no existing integration test that combines `fork-strategy` with an
ordered `tool.uv.environments` list. Nearby coverage in
`crates/uv/tests/lock/lock.rs` includes
`lock_requires_python_maximum_version` (default strategy chooses separate
latest compatible NumPy versions), `lock_requires_python_fewest_versions`
(`fewest` chooses one broadly compatible NumPy version), and
`lock_split_python_environment` (configured Python environments create the
expected lock split). None varies the configured environment order or combines
that order with both strategies.

Source inspection provides a plausible mechanism, but the behavioral
reproduction alone does not prove root cause: configured initial forks are
created in list order in
`crates/uv-resolver/src/resolver/environment.rs::initial_forked_states`, while
strategy-specific sorting in `crates/uv-resolver/src/resolver/mod.rs` is shown
for forks created later during dependency resolution. Completed fork solutions
also become preferences for subsequent forks.

## Classification

Bug. The documented contract says the default `requires-python` strategy
selects the latest compatible version for each supported Python version, while
`fewest` minimizes the number of versions. In the reproduction, changing only
the order of two disjoint `environments` entries changes both outcomes:
`requires-python` can fail to select NumPy 2.5.1 for Python 3.12, and `fewest`
can select two NumPy versions where one compatible version suffices.

The result is observed on the exact reported configuration with uv 0.12.3, so
no additional reporter information is needed to reproduce it. This is not
being classified from source inspection alone.

## Related

- astral-sh/uv#12782 is the closest adjacent open issue. It reports `fewest`
  failing to minimize versions with `required-environments`, which is a
  different setting and resolver path.
- astral-sh/uv#9998 reported incorrect `requires-python` selection when
  dependency-created forks were solved in the wrong order.
- astral-sh/uv#10007 fixed astral-sh/uv#9998 by prioritizing dynamically created
  forks according to `fork-strategy`.
- astral-sh/uv#9868 introduced `fork-strategy` and its two selection policies.
- astral-sh/uv#4662 introduced reuse of a completed fork's solution as
  preferences for subsequent forks, which is relevant to the observed
  `[preference]` selection but does not by itself confirm the root cause.
