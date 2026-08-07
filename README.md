# `fork-strategy` has no effect on the forks that come from `[tool.uv] environments`

Issue: astral-sh/uv#20999

Classification: bug

## Summary

astral-sh/uv#20999 reports that `tool.uv.fork-strategy` does not control forks
pre-seeded by `tool.uv.environments`. With Python 3.11 listed before Python 3.12,
the default `requires-python` strategy selects the Python 3.11-compatible NumPy
version for both forks; reversing the list allows Python 3.12 to select its
newest compatible version. The `fewest` strategy shows the inverse ordering
problem. Thus the order of the environment list overrides the selected strategy.

Repository documentation defines the intended behavior: `requires-python`
selects the latest compatible version for each supported Python version, while
`fewest` minimizes the number of selected versions. Current resolver source
turns `tool.uv.environments` entries into an ordered set of initial forks, pops
them in list order, and adds every completed fork's selected versions as
preferences for later forks. Strategy-specific prioritization exists for forks
created during dependency resolution, but not for this initial-fork path. The
report therefore identifies an implementation gap in existing behavior.

## Draft response

Thanks for the focused reproduction. This is a bug: `fork-strategy` should
determine whether uv prefers the latest compatible version per Python or the
fewest versions, and changing the order of `tool.uv.environments` should not
reverse that result.

The resolver currently creates those configured environments as initial forks
in list order, then reuses each completed fork's versions as preferences for the
following forks. The strategy-aware ordering added in astral-sh/uv#10007
applies to forks created later during dependency resolution, but not to these
initial forks. This is related to, but distinct from, astral-sh/uv#12782, which
covers `fewest` with `required-environments`.

The next step is to add coverage for both environment orders under both
strategies, then apply the strategy's fork prioritization to the initial forks
as well. The reproduction here is sufficient; no additional information is
needed.

## Classification

This is a bug, not an enhancement or question. The public multi-version
resolution documentation and astral-sh/uv#9868 establish that the setting
already promises the two selection policies. Source inspection confirms that
the reported order dependence follows from an uncovered code path: configured
environments become initial forks, those forks retain user order, and completed
solutions seed preferences for the remaining forks. Meanwhile, the sorting
that implements `ForkStrategy::RequiresPython` versus `ForkStrategy::Fewest`
is only applied to dependency-created forks.

It is not a duplicate. No open issue or pull request found in the search tracks
the same failure for `tool.uv.environments` initial forks. astral-sh/uv#12782 is
the closest open report, but its trigger is `required-environments`, which
affects required artifact coverage rather than supplying the ordered initial
forks involved here.

## Related issues and pull requests

- astral-sh/uv#12782 (open issue), “`fork-strategy` `fewest` does not limit
  versions when combined with `required-environments`”: the closest open
  adjacent report. It also shows `fewest` failing to minimize versions when
  environment coverage is configured, but it uses a different setting and
  resolver path.

- astral-sh/uv#9998 (closed issue), “fork-strategy requires-python produces
  unexpected results with repeated dependencies”: a historical instance in
  which `requires-python` did not produce the intended per-Python versions
  because dependency-created forks were solved in the wrong order.

- astral-sh/uv#10007 (merged pull request), “Prefer higher Python lower-bounds
  when forking”: fixed astral-sh/uv#9998 by ordering dynamically created forks
  according to `fork-strategy`. The current initial-fork path does not use this
  prioritization.

- astral-sh/uv#9868 (merged pull request), “Introduce a `--fork-strategy`
  preference mode”: introduced the setting and its intended contract—latest
  supported versions per Python by default, with `fewest` as the opt-out that
  minimizes selected versions.

- astral-sh/uv#4662 (merged pull request), “Set fork solution as preference
  when resolving”: introduced reuse of a completed fork's solution as
  preferences for later forks. This is the source-backed mechanism that makes
  the initial-fork order affect the selected versions.

## Search and evidence scope

Searches covered open and closed issues and open, closed, and merged pull
request history. Literal terms included `fork-strategy`,
`tool.uv.environments`, `required-environments`, `requires-python`, `fewest`,
`Solving split`, and resolver preferences. Conceptual searches covered initial
forks, fork ordering, preference reuse, minimizing versions, latest versions
per Python, and order-dependent lock output. Candidate comments and their
cross-reference chains were inspected, including astral-sh/uv#7190,
astral-sh/uv#4617, astral-sh/uv#4926, astral-sh/uv#5161, astral-sh/uv#5180,
astral-sh/uv#17866, and astral-sh/uv#19344.

astral-sh/uv#5161 was a plausible candidate because changing dependency order
changes its lock output, but it concerns general package-priority ambiguity and
does not establish the same `fork-strategy` violation. astral-sh/uv#17866 is
about retaining lockfile preferences when resolution options change, while
astral-sh/uv#19344 is about performance of repeated forks; neither tracks this
behavior.

## Supporting source evidence

- `docs/concepts/resolution.md` says the default strategy optimizes for the
  latest package version per supported Python version and `fewest` minimizes
  selected versions.
- `ResolverEnvironment::universal` documents that configured initial-fork order
  is significant, and `initial_forked_states` preserves that order for the
  resolver's stack.
- The resolver loop inserts the selected versions from every completed fork
  into shared preferences used by subsequent forks.
- The `ForkStrategy::Fewest` and `ForkStrategy::RequiresPython` sorting branch
  runs for `ForkedDependencies::Forked`, after the initial forks have already
  been created.
