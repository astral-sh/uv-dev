# Double `requires-python` exclusion gets ignored

Issue: astral-sh/uv#21036

Classification: bug

## Summary

The reporter's project supports Python 3.10 and 3.13 and declares:

```toml
requires-python = ">=3.10, !=3.11.*, !=3.12.*, <3.14"
```

With uv 0.12.3, `uv lock` writes `requires-python = ">=3.10, <3.14"` to
`uv.lock`, widening the supported set to include Python 3.11 and 3.12. A single wildcard
exclusion is retained. The reporter works around the problem by listing Python 3.10 and 3.13
explicitly in `[tool.uv].environments`.

This is a correctness bug in a previously fixed area, but the consecutive-wildcard trigger was
not covered by the closed fix. No open issue or pull request was found that already tracks this
case.

## Draft response

Thanks, this is a bug. The constraint is valid and `uv lock` should not widen it. This is in the
same conversion path previously addressed by astral-sh/uv#7862, astral-sh/uv#7897, and
astral-sh/uv#8060, but that work does not cover two consecutive wildcard exclusions: they become
one wider gap that the current conversion drops. Keeping this issue open for that uncovered case
is appropriate.

Your `[tool.uv].environments` setting is a valid workaround. The next implementation step is an
integration regression test for this exact constraint followed by preserving the gap during
workspace `requires-python` intersection.

## Classification

`bug` is the appropriate classification because the input is a valid PEP 440 constraint and the
serialized lockfile changes its meaning. The current source provides direct evidence for the
mechanism:

- `find_requires_python` collects workspace constraints and calls
  `RequiresPython::intersection` even for the ordinary project-lock path.
- `RequiresPython::intersection` converts the specifiers to ranges and reconstructs PEP 440
  specifiers with `VersionSpecifiers::from_release_only_bounds`.
- That reconstruction preserves a gap representing one excluded minor, but its fallback explicitly
  reports that it is ignoring an unsupported gap. Consecutive `!=3.11.*` and `!=3.12.*` exclusions
  form one gap from 3.11 through 3.12, which is wider than the single-minor case recognized by the
  conversion. The remaining bounding specifiers serialize as `>=3.10, <3.14`.

The closed astral-sh/uv#7862 and its fixes establish that losing or misrepresenting `!=` clauses
in this conversion is unintended. The new report is not a duplicate: the historical case was
closed after fixing a single exact hole, and the follow-up tests covered exact holes plus a
single wildcard-minor hole, not two consecutive wildcard-minor exclusions. There is no open
tracker for the uncovered case.

## Related issues and pull requests

### astral-sh/uv#7862 — `requires-python` specification not correctly resolved (closed)

This is the closest historical issue. In the same `uv lock` workflow, a valid
`>=3.8,<3.13,!=3.9.7` constraint was reconstructed as mutually contradictory `<3.9.7` and
`>3.9.7` clauses. A maintainer identified the conversion from a range with a hole back to PEP 440
specifiers as the cause. It is the same correctness area, but its one exact-version exclusion is
not the new consecutive-wildcard trigger.

### astral-sh/uv#7897 — Fix handling of != intersections in `requires-python` (merged)

This pull request closed astral-sh/uv#7862. It changed `RequiresPython::intersection` to avoid
turning exclusions into conjunctive lower and upper bounds and establishes that exclusion clauses
must survive workspace intersection. The initial approach retained the input clauses.

### astral-sh/uv#8060 — Add gap-preserving range-to-PEP 440 routine (merged)

This immediate follow-up replaced the initial fix with the conversion routine present in current
source. Its tests cover multiple exact exclusions and one wildcard-minor exclusion. Its code
preserves representable exact or single-minor holes but explicitly ignores an unsupported wider
gap. That limitation closely explains why the new pair of consecutive wildcard exclusions is
dropped.

### astral-sh/uv#20816 — `project.requires-python` not handled correctly (closed)

This is useful adjacent context, not a duplicate. A maintainer explained that commas are logical
AND and recommended `>=3.8,<3.12,!=3.10.*` for support with one skipped minor. The new reporter is
already using that correct form; their important additional condition is two consecutive skipped
minors.

## Search and evidence scope

GitHub searches covered open and closed issues and open, closed, and merged pull requests. Literal
queries included the exact `!=3.11.*` and `!=3.12.*` identifiers, `requires-python` with `!=`,
`uv.lock`, exclusion, ignored, simplified, wildcard, and the internal `Ignoring unsupported gap in
requires-python version` fragment. Conceptual searches covered disjoint, non-contiguous, skipped,
adjacent or consecutive Python versions, holes or gaps, range normalization/intersection, Python
support restrictions, and `[tool.uv].environments`. Fix-oriented searches covered merged pull
requests for `RequiresPython::intersection`, `from_release_only_bounds`, exclusion handling, and
gap-preserving conversion, including the issue and review chain from astral-sh/uv#7862 through
astral-sh/uv#7897 to astral-sh/uv#8060.

Several plausible results were inspected and ruled out. astral-sh/uv#6031 concerns `uv pip
compile --universal` not taking project `requires-python` metadata as its resolution bound, despite
a comment showing a similar environments workaround; it is a different command and behavior.
astral-sh/uv#6064 concerns irrelevant resolver forks and discarded output markers, not widening
the top-level lock constraint. astral-sh/uv#6176 is a broader resolver-fork design issue.
astral-sh/uv#15995 concerns `pylock.toml` deriving its bound from the active interpreter. None
tracks the consecutive exclusion round-trip reported here.
