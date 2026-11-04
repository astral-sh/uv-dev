# uv run --python 3.14 attempts to build locked NumPy 2.2.6 from source when a newer release has a compatible wheel

Issue: astral-sh/uv#23087

Classification: duplicate

## Summary

astral-sh/uv#9988 and astral-sh/uv#9515 cover the same cross-platform version reuse. astral-sh/uv#14588 requests adjacent selection controls; astral-sh/uv#8492 and astral-sh/uv#9827 explain the distinct historical Python-version fix.

The reporter runs `uv run -p 3.14 napari` from a fresh checkout of napari at revision
`8ad9a217b4dbcae0241907376cfef4d0edfcedae` on Windows 11 with uv 0.13.7
(`316c827e0`, November 3, 2026). They report that uv selects NumPy 2.2.6, which lacks a
Python 3.14 wheel, and attempts a source build even though newer NumPy releases offer wheels.
Python 3.13 runs, but also receives older NumPy and Numba versions.

A community commenter also reports the NumPy 2.2.6 selection on macOS ARM64 with uv 0.13.7
and Python 3.14.7. They report that limiting resolution to Apple Silicon selects newer NumPy
and Numba versions; the exact configuration and resulting versions were not supplied.

The reported trigger is an optional Numba requirement capped at 0.62.1 on macOS Intel.
The attached log supports cross-platform preference reuse: the resolver solves that platform
first, selects Numba 0.61.2 and NumPy 2.2.6, and subsequently selects those versions as
preferences in other platform branches. The attached run ends with a no-build installation
error, rather than showing an executed NumPy source build.

## Draft response

Your log shows the macOS Intel branch selecting Numba 0.61.2, which requires NumPy <2.3, then reusing NumPy 2.2.6 as a preference on other platforms. This matches the behavior discussed in astral-sh/uv#9988 and astral-sh/uv#9515: overlapping platform-specific ranges can share an older version.

The default strategy minimizes version differences across platforms. A missing wheel does not itself force an upgrade when a source distribution exists, and --python 3.14 selects the interpreter without restricting universal resolution to that environment. Your attached run ends with --no-build rejecting NumPy 2.2.6.

To require newer versions elsewhere, add platform/Python-specific lower bounds that exclude the older NumPy and Numba releases, then regenerate the lockfile. Please continue the per-platform version-selection discussion in astral-sh/uv#9988.

## Classification

The platform-specific upper bound holding other platforms to an older shared version is already discussed in astral-sh/uv#9988 and astral-sh/uv#9515. The supplied log confirms macOS Intel resolves first and NumPy 2.2.6 is subsequently selected as a preference elsewhere. Python 3.14 exposes the missing-wheel consequence of that same behavior. The historical fix in astral-sh/uv#9827 addresses Requires-Python lower-bound splitting, so the evidence does not establish a previously fixed bug returning.

The duplicate classification centralizes the existing platform-selection discussion; it does
not imply that selecting an unusable scientific-package version is desirable. No reproduction
was executed during this investigation; the macOS ARM64 reproduction is a community report.
The additional platform and reported resolution-scope workaround support the cross-platform
interpretation without establishing that the historical Python-version fix regressed.
astral-sh/uv#9988 was closed with a required-environments example for PyTorch, but that example
does not provide a general wheel-only guarantee for packages that also publish source
distributions. The continuing limitation here is the reuse of versions across overlapping
platform ranges.

## Related

- astral-sh/uv#9988 (issue, closed) — Dependency Resolver Ignores Platform-Specific Version Constraints. Closest match: a macOS Intel-only upper bound selects an older package across other platforms despite looser requirements there. Maintainers discuss disjoint version constraints. Its required-environments workaround concerns wheel-only packages and does not guarantee compatible wheels for NumPy, which publishes source distributions.
- astral-sh/uv#9515 (issue, closed) — Specifying different dependency versions for different platforms. Predecessor to astral-sh/uv#9988 with the same overlapping platform-specific ranges. A maintainer explicitly confirms that uv reuses the older version when it satisfies both ranges and recommends disjoint bounds to require different versions.
- astral-sh/uv#14588 (issue, open) — New `--fork-strategy` option to prioritize the latest version for each Python implementation. Adjacent request for independently selecting newer versions instead of sharing an older version across environments. It concerns CPython versus PyPy, whereas this report concerns operating-system/architecture branches and a transitive NumPy constraint.
- astral-sh/uv#8492 (issue, closed) — Fork when wheels only support a specific range. Historical NumPy source-build symptom on newer Python when older-Python support held back dependency versions. Closed by astral-sh/uv#9827; the implemented fix splits on Requires-Python lower bounds, unlike this report's platform-driven version reuse.
- astral-sh/uv#9827 (pull request, merged) — Fork version selection based on `requires-python` requirements. Merged December 13, 2024, fixing astral-sh/uv#8492 by forking when newer releases raise their Python minimum. It does not implement independent latest-version selection for every platform or infer Python upper compatibility from missing wheels; this report does not establish a regression of that fix.

## Supporting evidence

### Report and dependency declarations

The event payload and issue body agree. The supplied full discussion includes one community
comment with an additional affected platform and a reported resolution-scope workaround.
The reporter's linked napari `pyproject.toml` declares `requires-python = ">=3.11"`,
`numpy>=1.25`, and `scipy>=1.14.0`. The `optional-numba` extra declares:

```toml
"numba>=0.58.0,<=0.62.1 ; platform_system == 'Darwin' and platform_machine == 'x86_64'",
"numba>=0.58.0 ; platform_system != 'Darwin' or platform_machine != 'x86_64'",
```

These Numba ranges overlap. The project does not declare a `tool.uv` table at that revision.
A request for `uv.lock` at that revision returned an HTTP error, so a committed lockfile
was not available to inspect. The supplied log directly shows resolution, rather than
establishing that an old committed lockfile alone caused the selection.

Source: https://raw.githubusercontent.com/napari/napari/8ad9a217b4dbcae0241907376cfef4d0edfcedae/pyproject.toml

### Additional affected platform and reported workaround

A community comment by sainimal1ba-hue on astral-sh/uv#23087 (November 4, 2026) reports:

- On macOS ARM64, uv 0.13.7 with Python 3.14.7 selects NumPy 2.2.6 when running
  `uv run -p 3.14 napari`, despite the host not being an Intel Mac.
- Limiting resolution to Apple Silicon selects newer NumPy and Numba versions. This is a
  reported workaround for a narrower target environment, not a demonstrated solution for a
  project that needs to retain its full platform support.

The commenter did not provide the exact resolution-limiting configuration, replacement package
versions, a new verbose log, or an explicit checkout revision for their test. Their attribution
to the Intel-only Numba cap combined with testing constraints is an interpretation; the comment
does not identify those testing constraints or independently establish their contribution.
The original attachment and source analysis remain the evidence for preference reuse.

This report extends the affected hosts beyond Windows and supplies a useful comparison for
investigation: universal resolution versus resolution limited to Apple Silicon. It has not been
independently reproduced during this investigation. The commenter asks whether a resolver fix
would be welcome, but the supplied discussion contains no maintainer decision authorizing one.

### Attached resolver log

The relevant sequence in the reporter's verbose attachment is:

1. Solve the macOS Intel split: `platform_machine == 'x86_64' and sys_platform == 'darwin'`.
2. Consider newer NumPy releases, then select Numba 0.61.2 with
   `numpy>=1.24, <2.3`, and select NumPy 2.2.6.
3. Solve the complementary platform split and select NumPy 2.2.6 with
   `[preference]`; Numba 0.61.2 is also reused as a preference in subsequent branches.
4. Finish with:
   `Distribution numpy==2.2.6 ... can't be installed because it is marked as --no-build but has no binary distribution`.

The attachment establishes the preference reuse and the wheel rejection. It does not independently
demonstrate the performance impact mentioned in the report, nor does it show a completed source build.

Source: https://github.com/user-attachments/files/34343028/uv_out.txt

### Repository documentation and source

- `docs/concepts/resolution.md:407` describes the default `requires-python` fork strategy:
  prefer newer releases for each supported Python version while minimizing versions across
  platforms. It is not an independent latest-version strategy for every operating system.
- `docs/concepts/resolution.md:175` explains source fallback when a selected release lacks
  a compatible wheel. The required-environments discussion specifically explains the default
  wheel requirements for packages without source distributions.
- `docs/concepts/resolution.md:123` describes universal resolution over the project's declared
  Python range. Selecting Python 3.14 for execution does not reduce that range to one interpreter.
- `crates/uv-resolver/src/resolver/mod.rs:793` orders dependency-created forks by Python bounds
  and then upper-bound constraints. The `cmp_upper_bounds` implementation at line 4077
  explicitly favors solving upper-bounded requirements before overlapping, less restrictive ones.
- `crates/uv-resolver/src/resolver/mod.rs:497` records selected versions from completed forks
  as resolver preferences. This supports the interpretation of the attachment's
  `[preference]` selections.
- `docs/concepts/projects/sync.md:157` explains that existing lockfile versions are preferred
  and upgrades remain constrained by declared requirements. An upgrade alone is not a guarantee
  of independently newer versions across platforms.

The November 2026 report postdates the December 2024 merge of astral-sh/uv#9827. The distinction
is behavioral, not merely chronological: that PR introduced splits at higher Requires-Python
minimums, while the present log shows platform constraints and preference reuse.

## Search coverage and excluded candidates

Authenticated gh searches covered open and closed issues and open, closed, and merged pull requests. Literal searches included NumPy/Python 3.14, NumPy 2.2.6, Numba, Darwin, no-build, and the 'marked as --no-build'/'no binary distribution' error fragments. Separate conceptual searches covered platform markers, overlapping constraints, universal resolution, cross-fork preferences, fork-strategy, latest-per-platform selection, and wheel versus source fallback; fix-oriented searches covered Requires-Python forking and newer-Python wheel selection. PR searches returned no matches, so linked PRs were inspected directly, including astral-sh/uv#8686, astral-sh/uv#9827, and astral-sh/uv#14041. Followed maintainer comments and reference chains through astral-sh/uv#9515, astral-sh/uv#9988, and astral-sh/uv#9711. Ruled out astral-sh/uv#17649 and astral-sh/uv#12060 as different backtracking-priority cases, astral-sh/uv#20999 as explicit-environments ordering, and astral-sh/uv#19911 as missing index Requires-Python metadata. Checked the reported napari revision, attached log, repository documentation, and resolver source.

Additional candidates were inspected to distinguish similar symptoms:

- astral-sh/uv#17649 and astral-sh/uv#12060 keep a newer NumPy while backtracking Numba/llvmlite
  into old releases. This report instead shows a platform cap selecting an older NumPy and then
  carrying it to other platforms.
- astral-sh/uv#20999 concerns ordering of explicit `tool.uv.environments` entries.
  The reported napari revision has no such configuration.
- astral-sh/uv#19911 concerns a custom index omitting Requires-Python metadata and incorrectly
  rejecting a candidate. The present evidence shows successful selection followed by preference reuse.
- astral-sh/uv#9711 concerns incomplete wheel-only resolutions and links platform-related
  discussions, but does not directly describe NumPy's source-distribution fallback.
- astral-sh/uv#13332 concerns `uv pip compile --universal`, `--only-binary`, and
  required-environments behavior; the command and configuration differ here.
- astral-sh/uv#16219 is a Python 3.14 build-toolchain support case without this platform-cap trigger.
- astral-sh/uv#9998 concerns repeated dependency declarations disrupting Python-version forking.
  The shared platform-version behavior is the closer match.
- astral-sh/uv#14041 fixes the implied Windows AMD64 marker. The supplied log already separates
  macOS Intel and its complement, so that historical marker fix is not established as the cause.

## Scope and validation

This handoff is based on read-only GitHub queries, the reporter's attachment, the referenced napari
metadata, the checked-out uv documentation/source, and the supplied issue/comment snapshots.
The supplied full discussion includes the new comment; the older issue-context snapshot has none.
The comment adds user-reported findings, not an independently executed reproduction.
No packages were installed and no builds or
tests were run. No files in the checkout or GitHub objects were changed. Only this temporary
issue-context README was updated; unrelated existing checkout changes were left untouched.
