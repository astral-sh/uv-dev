# uv sync --upgrade does not upgrade a transitive dependency (setuptools)

Issue: astral-sh/uv#21273

Classification: question

## Summary

The reporter is using uv 0.12.5 on Ubuntu with Python 3.14.4. Running
`uv sync --upgrade`, including with `--resolution highest`, leaves setuptools at 81.0.0 in
`uv.lock`. Setuptools is not a direct runtime dependency: it is reached through Pyramid and is also
the project's build-system requirement.

This is expected for the reported dependency graph. Pyramid 2.1 publishes a runtime requirement of
`setuptools<82`, and setuptools 81.0.0 is therefore the highest compatible version. The project has
no direct setuptools requirement that relaxes that transitive upper bound. uv's documented upgrade
behavior remains subject to the dependency graph's constraints, and `--resolution highest` means
the highest *compatible* version rather than the highest published version.

Repository member zanieb confirmed this explanation directly in astral-sh/uv#21273: Pyramid 2.1's
`setuptools<82` requirement makes 81.0.0 the highest version uv can select.

The reporter's follow-up shifts the remaining question to discoverability: how a user can identify
the transitive constraint responsible for holding a package back without searching hundreds of
verbose resolver log lines.

The behavior was reproduced with the reported uv and Python versions in a minimal project. The
lockfile remained byte-for-byte unchanged after the upgrade. An explicit request for the current
setuptools 84.0.0 failed with a resolver explanation identifying Pyramid's `setuptools<82`
requirement. This confirms the constraint as the reason for the observed result; it is not evidence
that uv skipped a transitive dependency.

No existing issue or pull request tracks a uv defect matching this exact report. The closest prior
issues cover the same constraint rule and a different transitive-upgrade capability.

## Reproduction

Outcome: **reproducible**, as expected resolver behavior rather than a uv defect.

Environment: uv 0.12.5 (`x86_64-unknown-linux-gnu`), CPython 3.14.4 managed by uv, and Ubuntu
24.04.4 x86_64. All project files, the virtual environment, Python installation, and uv cache were
isolated in a new directory under `/tmp`.

The linked project's current `pyproject.toml` declares Python `>=3.12`, an unversioned `pyramid`
runtime dependency, `pyramid>=2.0.2` in the default `dev` group, and unversioned `setuptools` in
`build-system.requires`. Its lockfile selects Pyramid 2.1 and setuptools 81.0.0. The relevant
runtime edge is present in Pyramid 2.1's published metadata as `setuptools<82`.

The minimal project was:

```toml
[project]
name = "issue-21273-reproduction"
version = "0.1.0"
requires-python = "==3.14.4"
dependencies = ["pyramid==2.1"]

[tool.uv]
package = false
```

Using isolated `UV_CACHE_DIR` and `UV_PYTHON_INSTALL_DIR` paths, the targeted commands were:

```console
$ uv sync --python 3.14.4
...
+ pyramid==2.1
+ setuptools==81.0.0
$ cp uv.lock uv.lock.before
$ uv sync --upgrade --python 3.14.4
Resolved 13 packages ...
Checked 12 packages ...
$ cmp -s uv.lock uv.lock.before
# exit status 0: the lockfile is unchanged
$ uv sync --upgrade --resolution highest --python 3.14.4
Resolved 13 packages ...
Checked 12 packages ...
$ cmp -s uv.lock uv.lock.before
# exit status 0: the lockfile is still unchanged
```

`uv tree` showed setuptools 81.0.0 below Pyramid 2.1. A constraint probe made the reason explicit:

```console
$ uv lock --upgrade-package setuptools==84.0.0
× No solution found when resolving dependencies:
╰─▶ Because pyramid>=2.1 depends on setuptools<82 and setuptools==84.0.0, we
    can conclude that pyramid>=2.1 cannot be used.
```

Thus the reported unchanged setuptools 81.0.0 pin is reproducible, but it is the highest version
allowed by Pyramid 2.1. The build-system requirement and default dependency group do not remove the
runtime upper bound.

## Constraint visibility

For a synced environment, uv already has a focused way to display this edge:

```console
$ uv pip tree --show-version-specifiers --invert --package setuptools
setuptools v81.0.0
└── pyramid v2.1 [requires: setuptools <82]
```

The exact tree can include other reverse dependencies, but `--show-version-specifiers` exposes the
declared bounds and `--invert --package setuptools` narrows the output to packages requiring
setuptools. This is substantially more targeted than `uv sync --upgrade -v`, where the reporter
found the same `setuptools<82` edge among more than 450 debug lines.

The project-lockfile command `uv tree` does not currently support `--show-version-specifiers`.
astral-sh/uv#9059 is the open canonical request for parity with `uv pip tree`; maintainers note that
transitive specifiers are not stored in `uv.lock`, so implementing it may require expanding the
lockfile or resolving metadata again. The installed-environment flag was requested in
astral-sh/uv#5217 and implemented by merged pull request astral-sh/uv#5240.

Repository member zanieb has opened astral-sh/uv-dev#847 as an implementation sketch for the
project-tree option. The draft annotates dependency edges using lockfile metadata when available
and retrieves missing metadata for displayed locked packages only when the flag is requested. This
approach does not change the lockfile format and respects display filtering such as depth limits
and deduplication. Its initial scope is text output of individual requirements; it does not combine
constraints or explain why the resolver selected a particular version. The draft is open with CI
passing, but the author explicitly gave no timeline for advancing it.

The reporter also linked upstream context. Pylons/pyramid#3795 proposed the setuptools bound as a
temporary response to `pkg_resources` removal, but that pull request is closed and was not merged.
The broader `pkg_resources` migration remains open in Pylons/pyramid#3731, where current discussion
confirms that replacement work is ongoing. These links explain why Pyramid needs the bound but do
not change uv's resolver behavior.

## Workaround and maintainer guidance

Repository member zanieb stated that Pyramid ideally would not impose this upper bound and pointed
to uv's dependency overrides as an escape hatch. For example, a project that has independently
confirmed compatibility with newer setuptools versions can replace the declared bound with:

```toml
[tool.uv]
override-dependencies = ["setuptools>=83"]
```

This is a deliberate override of package metadata, not a normal upgrade. uv's documentation
describes overrides as a last resort for cases where compatibility beyond a declared bound is
known; without that validation, waiting for Pyramid to relax the requirement remains the safer
course.

## Draft response

Pyramid 2.1 declares `setuptools<82`, so setuptools 81.0.0 is the newest version compatible with
this dependency graph. `--upgrade` and `--resolution highest` still respect package requirements
and therefore cannot select setuptools 83 or newer.

To use a newer setuptools, you'll need a Pyramid release that relaxes that bound. An override is
also possible, but it should only be used after confirming that Pyramid is compatible with the
newer setuptools release.

## Classification

This is a question rather than a bug. The linked project's `uv.lock` contains Pyramid 2.1 and
setuptools 81.0.0, while Pyramid 2.1's published metadata requires `setuptools<82`. The repository
documentation states that upgrades are limited by dependency constraints. Consequently, retaining
81.0.0 is correct resolver behavior, even for a global upgrade and highest resolution. The
targeted reproduction confirmed this behavior with uv 0.12.5 and Python 3.14.4.

The report's suggestion that transitive dependencies might be skipped is not supported by the
evidence. The build-system requirement is also not the blocker: the runtime dependency edge from
Pyramid imposes the upper bound.

## Related

- astral-sh/uv#12655 — Closed question where `uv lock --upgrade` could not update packages because
  project constraints excluded newer versions. Maintainers confirmed the same governing rule. The
  difference is that its constraints were direct exact pins, while astral-sh/uv#21273 is blocked by
  Pyramid's transitive upper bound.
- astral-sh/uv#14213 — Closed question about upgrading every transitive dependency of one selected
  parent. Maintainers explained that uv supports upgrading one named transitive package or all
  packages, but not selecting all transitives of one parent. In astral-sh/uv#21273, a global upgrade
  is already requested; the package remains unchanged because of a constraint instead.
- astral-sh/uv#9059 — Open request for `uv tree` to support `--show-version-specifiers`, directly
  covering the follow-up request to make transitive upper bounds visible from project lock data.
- astral-sh/uv#5217 — Closed request for version-specifier display in `uv pip tree`, motivated by
  diagnosing why a dependency remains on an older version.
- astral-sh/uv#5240 — Merged pull request implementing `uv pip tree --show-version-specifiers`, the
  currently available workflow for inspecting constraints in the synced environment.
- astral-sh/uv-dev#847 — Open draft implementation sketch for adding
  `--show-version-specifiers` to project `uv tree` without expanding the lockfile format. It directly
  addresses the follow-up discoverability request, but has no stated completion timeline.

No closely matching pull request was found for the original resolver report; astral-sh/uv#5240 and
astral-sh/uv-dev#847 relate specifically to the follow-up discoverability question.
astral-sh/uv#11784 was inspected because it also mentions `uv sync --resolution highest`, but that
command did not include an upgrade request and correctly retained lockfile preferences.
astral-sh/uv#18178 was also inspected because both a targeted and global upgrade appeared
ineffective, but its cause was a configured `lowest-direct` resolution mode; the reporter here
explicitly tried `highest`. Neither is the same case.

Searches covered the literal command and package terms (`uv sync --upgrade`, setuptools, Pyramid,
`--resolution highest`, transitive/indirect dependency), conceptual terms (locked versions,
global and targeted upgrades, latest compatible versions, dependency constraints and upper
bounds), and fix-oriented searches across open and closed issues plus open, closed, and merged pull
requests. Searches also removed the package and platform details to look for the underlying
constraint behavior. The follow-up search also covered displaying constraints and version
specifiers in `uv tree` and `uv pip tree`, identifying astral-sh/uv#9059 as the canonical open
request. No version-specific regression or matching resolver fix was found.
