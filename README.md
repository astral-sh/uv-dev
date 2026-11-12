# uv does not install requirement `eccodeslib` when `eccodes` is requested

Issue: astral-sh/uv#23222

Classification: duplicate

## Summary

The reporter says requesting `eccodes` with uv does not install its runtime dependency
`eccodeslib`, whereas pip does. Explicitly adding `eccodeslib` works. The reported environment is
Darwin 25.6.0 ARM64, Python 3.11.15, and uv 0.11.28
(`ebf0f43d7`, dated 2026-07-07). The issue does not provide an exact command, resolved
`eccodes` version, lockfile, or verbose output.

ecmwf/eccodes-python#150 directly tracks the eccodeslib omission. astral-sh/uv#10693 covers the same inconsistent-wheel-metadata failure mode; astral-sh/uv#7381 was an unmerged proposal relevant to wheel selection.

The reporter linked ecmwf/eccodes-python#150. That open upstream issue describes the omission
beginning with `eccodes` 2.45.0, includes a macOS reproduction, and contains the same
pip-versus-`uv add` observation. Public PyPI metadata independently confirms inconsistent
requirements across wheels for versions 2.45.0 and 2.46.0.

Maintainer charliermarsh also supplied a version-scoped dependency metadata override as an
alternative to explicitly adding `eccodeslib`; see the workaround below. Its example version,
2.50.0, does not establish the reporter's resolved version.

## Draft response

This matches ecmwf/eccodes-python#150 and the resolver behavior discussed in astral-sh/uv#10693. For eccodes 2.45.0 and 2.46.0, the platform-independent wheel declares eccodeslib on non-Windows systems, but the Windows wheels omit that requirement entirely. uv's universal resolver reads one wheel's metadata and assumes requirements are consistent across a release, so differing wheel metadata can leave dependencies out.

Please follow ecmwf/eccodes-python#150 for the package metadata issue. You can continue using `uv add eccodeslib` as a workaround. To confirm this explains your environment, please share your exact install command and resolved eccodes version.

## Classification

ecmwf/eccodes-python#150 already tracks the same package, missing dependency, pip-versus-uv behavior, and explicit-add workaround; astral-sh/uv#10693 tracks the same universal-resolution failure mode caused by inconsistent wheel requirements. Artifact metadata and uv's documented consistency assumption support this relationship. The reporter's precise eccodes version and command remain unspecified, so the wheel selected in this environment is unconfirmed. No evidence establishes a regression of a previously fixed uv bug; astral-sh/uv#18373 fixes a different, cross-index problem.

The duplicate classification takes precedence over a separate bug or question label because an
existing upstream issue covers this exact omission, with a matching open uv discussion for the
underlying resolver behavior. A documented resolver limitation explains the mechanism; it does
not make a missing required runtime dependency correct.

## Related

- ecmwf/eccodes-python#150 — 2.45.0 (+ and 2.46) release misses eccodeslib dependency - "Cannot find the ecCodes library" (open issue). Direct upstream match: eccodes omits eccodeslib under uv, explicitly adding eccodeslib works, and comments include the same pip-versus-uv difference and a macOS reproduction. Independently inspected PyPI wheel metadata for eccodes 2.45.0 and 2.46.0 confirms that Windows wheels omit eccodeslib while platform-independent wheels require it on non-Windows systems.
- astral-sh/uv#10693 — PyTorch nightly inconsistency between `uv pip install` vs `uv sync` (missing nvidia packages) (open issue). Same resolver failure mode with another package: universal locking omits runtime dependencies that single-platform installation includes. Maintainer analysis confirms different Requires-Dist metadata between wheels of one release, selection of one wheel for universal resolution, and explicitly adding missing dependencies as a workaround.
- astral-sh/uv#7381 — Prefer compatible wheels in universal resolver (closed pull request). Historical proposal to prefer the current platform's wheel when reading metadata, directly relevant to the pip-versus-universal-resolution difference. Closed without merging because inconsistent wheel metadata would make resolution platform-dependent; this was not a shipped fix whose failure would establish a regression.

## Metadata override workaround

In astral-sh/uv#23222, maintainer charliermarsh recommends overriding `eccodes` dependency
metadata to include the missing non-Windows requirement alongside its base requirements. The
example uses version 2.50.0. In `pyproject.toml`, the documented table form is:

```toml
[[tool.uv.dependency-metadata]]
name = "eccodes"
version = "2.50.0"
requires-dist = [
    "numpy",
    "attrs",
    "cffi",
    "findlibs",
    "eccodeslib; platform_system != 'Windows'",
]
```

`docs/concepts/resolution.md`, under Dependency metadata, documents this mechanism for incorrect
or incomplete package metadata. The override supplies the package's dependency list, so the
base requirements must be included along with `eccodeslib`. This entry applies only to
`eccodes` 2.50.0; another resolved version needs matching metadata rather than assuming the
example applies unchanged. The Windows marker retains the platform condition found in the
previously inspected platform-independent wheels.

This is a maintainer-provided alternative to the reporter's working `uv add eccodeslib`
approach. The override has not been reproduced locally or confirmed by the reporter. Neither
the example nor the new comment confirms that the reporter uses `eccodes` 2.50.0 or establishes
that version's published wheel metadata.

## Supporting evidence

### Published package requirements

The following metadata was read directly from public PyPI artifact `.metadata` endpoints.
No package code was installed or executed.

| Release | Windows wheel inspected | Platform-independent wheel inspected |
| --- | --- | --- |
| 2.45.0 | `eccodes-2.45.0-cp310-cp310-win_amd64.whl`: `numpy`, `attrs`, `cffi`, `findlibs`; no `eccodeslib` | `eccodes-2.45.0-py3-none-any.whl`: same base requirements plus `eccodeslib; platform_system != "Windows"` |
| 2.46.0 | `eccodes-2.46.0-cp310-cp310-win_amd64.whl`: same four base requirements; no `eccodeslib` | `eccodes-2.46.0-py3-none-any.whl`: same base requirements plus `eccodeslib; platform_system != "Windows"` |

Release-level PyPI JSON also omits `eccodeslib` for these versions. The artifact-level mismatch,
rather than an assumption that uv reads the release-level JSON API, supports the explanation.

The upstream `setup.py` adds `eccodeslib;platform_system!='Windows'` only when
`DEPEND_ON_ECCODESLIB=1`. ecmwf/eccodes-python#141, merged on 2025-10-21, introduced
different dependency behavior for source distributions; its maintainer comments expected normal
wheel installation to bring in `eccodeslib`. This is supporting packaging history, not evidence
of a uv fix.

### Resolver behavior and maintainer conclusions

- `docs/reference/internals/resolver.md:156` states that wheels of one version within a
  specific index must have the same `Requires-Dist`, including wheels built from source
  distributions. It explains why uv reads one distribution's metadata rather than every wheel.
- `crates/uv-resolver/src/prioritized_distribution.rs:601` explicitly documents the assumption
  that distributions of one version have the same metadata when choosing a wheel to obtain
  metadata for a source distribution.
- In astral-sh/uv#10693, maintainer zanieb compares wheel metadata and confirms universal locking
  uses one wheel whose requirements can differ from those of the installed wheel. The discussion
  follows astral-sh/uv#10119, where charliermarsh identifies the same inconsistency and recommends
  correcting the upstream metadata or selecting a specific wheel.
- In astral-sh/uv#12588, charliermarsh describes metadata consistency as a fundamental resolver
  requirement and suggests static dependency metadata as a workaround. That report fails
  resolution rather than silently omitting a dependency, so it is supporting context.
- astral-sh/uv#7381 was closed without merging on 2025-02-11. The author cites loss of consistent
  cross-platform resolution as the reason. A later maintainer comment in astral-sh/uv#5921
  reiterates that determinism takes priority.

### Candidates ruled out and regression assessment

- astral-sh/uv#9412 and astral-sh/uv#12205 mention `eccodes`, but fail because a selected release
  has no usable wheel or source distribution for the current platform. Required environments
  address that situation; these reports do not describe an omitted transitive dependency.
- astral-sh/uv#17065 concerns dependencies conditioned on negative extra markers in a
  Poetry-backed editable package, rather than requirements varying across published wheels.
- astral-sh/uv#14237 initially resembles the omission, but maintainers traced it to sharing
  metadata across indexes. astral-sh/uv#18373, merged on 2026-03-10, changed the metadata cache key
  to include distribution/index identity. That fix predates the reported uv build but does not
  address differing wheel metadata within PyPI, so it does not establish a regression here.
- astral-sh/uv#5845, merged on 2024-08-07, prevents reuse of incompatible distributions during
  build-dependency resolution. It does not fix the reported cross-wheel requirements mismatch.
- astral-sh/uv#11463 requests excluding foreign-platform wheels to avoid proxy HTTP 403 errors;
  it does not track missing runtime requirements.

## Search scope

Searched astral-sh/uv open and closed issues and open, closed, and merged pull requests with authenticated gh. Literal searches covered eccodes, eccodeslib, the upstream error Cannot find the ecCodes library, and uv 0.11.28. Separate symptom searches covered missing or uninstalled transitive dependencies and pip versus uv add/sync; conceptual searches covered universal resolution, conditional dependencies, inconsistent wheel metadata, differing Requires-Dist, and platform-compatible wheel selection. Followed maintainer references through astral-sh/uv#10693, astral-sh/uv#10119, astral-sh/uv#5921, astral-sh/uv#7381, and the reporter's upstream lead. Inspected astral-sh/uv#12588 for the metadata-consistency limitation. Ruled out same-package astral-sh/uv#9412 and astral-sh/uv#12205 because they concern unavailable platform wheels, and astral-sh/uv#17065 because it concerns negative extra markers. Fix-oriented review ruled out astral-sh/uv#14237 and merged astral-sh/uv#18373, which address metadata contamination across indexes rather than different wheels within one index; merged astral-sh/uv#5845 addresses incompatible build distributions. Also inspected astral-sh/uv#11463, which concerns proxy 403 errors, not missing requirements. Checked resolver documentation/source and public PyPI wheel metadata for eccodes 2.45.0 and 2.46.0.

## Remaining information

The exact `eccodes` version, install command, and metadata artifact chosen in the reporter's
environment remain unconfirmed. Ask for the command and resolved version to verify the match.
The suggested `uv add eccodeslib` workaround matches both the report and the upstream discussion.
The maintainer-provided metadata override is another option, with configuration and validation
limits described above; its example version does not resolve the missing version information.
No evidence supports claiming that uv uses PyPI's release-level JSON as its metadata source, or
that a newer uv release has fixed this behavior.
