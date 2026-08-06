# uv add --extra installs from pypi if dependency group is labelled like a public package

Issue: astral-sh/uv#20994

Classification: question

## Summary

The reporter added a local package to a workspace development dependency group with:

```console
uv add --dev private-package/ --extra dev --extra test --extra future
```

The local package declares `dev`, `test`, and `future` optional dependencies, with `future`
containing two private packages. uv correctly recorded the request as
`private-package[dev,future,test]`, but the reporter also observed installation of the unrelated
PyPI distribution named `future`. Removing the `future` extra and running `uv sync` reportedly
removed both that distribution and the private packages activated by the extra.

The reported same-name installation could not be produced from the supplied configuration. The
extra name alone did not create a dependency on the same-named PyPI distribution, but the report
does not include enough of the original dependency graph and source configuration to determine
which edge introduced `future`. The reproduction outcome is therefore `needs_more_information`.

## Reproduction

Outcome: `needs_more_information`.

The targeted reproduction ran on Linux x86_64 with Python 3.12.3. It was tested with the reported
uv 0.12.1 and the installed uv 0.12.2. All project files, environments, Python-install directories,
and caches were isolated under `/tmp`.

The fixture was a root workspace with three local members. `private-package` declared:

```toml
[project.optional-dependencies]
dev = []
test = []
future = [
  "private-package-1",
  "private-package-2",
]

[tool.uv.sources]
private-package-1 = { workspace = true }
private-package-2 = { workspace = true }
```

From the workspace root, the reported command was run unchanged:

```console
uv add --dev private-package/ --extra dev --extra test --extra future
```

On both uv versions, uv resolved four workspace packages and installed only
`private-package==0.1.0`, `private-package-1==0.1.0`, and `private-package-2==0.1.0`. The generated
root dependency group was:

```toml
[dependency-groups]
dev = [
    "private-package[dev,future,test]",
]
```

The lockfile contained only the root and the three local workspace packages. `uv pip list` did not
contain `future`, and `uv pip show future` reported `Package(s) not found for: future`. Thus, a
selected optional dependency named `future` was observed to activate its declared members without
being converted into a distribution requirement of the same name.

As a second check on the cleanup behavior, the root requirement was changed to
`private-package[dev,test]` and `uv sync` was run with uv 0.12.1. uv uninstalled only
`private-package-1` and `private-package-2`; `private-package` remained installed, and no public
`future` distribution had been present.

The closest existing integration coverage is
`crates/uv/tests/project/edit.rs`, test `update`. It passes repeated `--extra` flags to `uv add` for
`requests`, asserts that the extras are written onto the `requests[...]` requirement, and observes
installation of the dependencies supplied by those extras. It does not cover a local path or
workspace dependency whose selected extra collides with a registry distribution name, so it is
related behavior rather than coverage of this exact report.

The complete root and member `pyproject.toml` files, index and source configuration, and the
relevant original `uv.lock` dependency entries are essential missing inputs. A useful next report
should include sanitized versions of those files plus:

```console
uv tree --invert --package future
```

If the tree output cannot be shared, the `[[package]]` block for `future` and every lockfile entry
that refers to it would identify the dependency edge. Without that information, the simplified
fixture behaving correctly is not sufficient evidence to classify the original observation as
`not_reproducible`.

## Draft response

`--extra future` enables the `future` extra on `private-package`; it does not request a separate
distribution named `future`. The generated `private-package[dev,future,test]` entry is expected.
With uv 0.12.1 and 0.12.2, a workspace fixture in which that extra contains two local packages
installed only those packages and did not install the PyPI `future` distribution.

Please share `uv tree --invert --package future` and a sanitized complete reproducer, including the
root and member `pyproject.toml` files, source/index configuration, and the relevant `uv.lock`
entries. That will show which dependency metadata introduced `future`. Removing the extra and
running `uv sync` is expected to remove dependencies that were reachable only through that extra,
as discussed in astral-sh/uv#11937.

## Classification

This remains a `question`: astral-sh/uv#20994 asks whether the observation is intended, while the
available configuration does not reproduce or establish an incorrect uv dependency edge. The
observed command semantics are consistent with the CLI definition of `--extra`: it enables extras
for the dependency being added, while `--optional` adds a dependency to one of the current
project's optional extras.

## Related issues and pull requests

- astral-sh/uv#9011 demonstrates placing `package[extra]` in a dependency group to activate the
  package's published optional dependencies. It supports the generated requirement representation
  but does not report a same-named distribution being installed.
- astral-sh/uv#11937 explains exact sync and why packages activated only by a removed extra are
  removed. It covers the cleanup portion of the report, not the origin of `future`.
- astral-sh/uv#18965 identifies `uv tree --invert --package <package>` as the command for tracing a
  package back to its dependents.

No related pull request was close enough to include. Historical fixes in astral-sh/uv#6395 concern
invalid extra names beginning with `_` or `-`, not the valid and explicitly selected `future` extra
in astral-sh/uv#20994.
