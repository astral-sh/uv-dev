# uv add --extra installs from pypi if dependency group is labelled like a public package

Issue: astral-sh/uv#20994

Classification: question

## Summary

The reporter added a local package to the development dependency group with:

```console
uv add --dev private-package/ --extra dev --extra test --extra future
```

The local package declares a `future` optional dependency whose members are two private packages. uv correctly recorded the request as `private-package[dev,future,test]`, but the reporter also observed installation of the unrelated PyPI distribution named `future`. Removing `future` from that requirement and running `uv sync` removed both the private packages activated by the extra and the public `future` distribution.

No existing issue tracks the same claimed name-collision behavior. The available evidence establishes that the generated `package[extra]` requirement and the later exact-sync removals are expected, but it does not establish why the public distribution entered this dependency graph.

## Draft response

`--extra future` in this command enables the `future` extra on `private-package`; it does not request a separate distribution named `future`. The generated `private-package[dev,future,test]` entry is expected. After removing that extra, `uv sync` performs an exact sync and removes packages that are no longer reachable, as described in astral-sh/uv#11937.

A PyPI distribution named `future` should not be introduced solely because the extra has that name. Please share the output of `uv tree --invert --package future` and a sanitized complete reproducer, including the relevant `pyproject.toml` files or the `future` package and dependency entries from `uv.lock`. That will show which dependency metadata introduced `future` and whether uv formed an incorrect dependency edge.

## Classification

This is a `question` because the report primarily asks whether the observed behavior is intended, and the currently supported evidence does not establish an incorrect uv dependency edge.

The expected behavior is source-backed:

- The `uv add` CLI describes `--extra` as enabling extras for the dependency being added and directs users to `--optional` when they instead want to add a dependency to an optional extra.
- The project dependency documentation defines extras through `package[extra]`; therefore the generated `private-package[dev,future,test]` entry is the intended representation.
- Maintainer comments in astral-sh/uv#9011 demonstrate the same structure by placing `example[feat]` in a dependency group to activate `example`'s `feat` optional dependency.
- Maintainer comments in astral-sh/uv#11937 explain that `uv sync` computes the selected dependency set and removes everything else. Removing the `future` extra therefore should remove dependencies reachable only through that extra.

A minimal reproduction with uv 0.12.1 used a fresh root project and a local `private-package` declaring empty `dev`, `test`, and `future` extras. Running the reported `uv add` command produced the expected `private-package[dev,future,test]` group entry and a two-package lockfile containing only the root and local package; it did not introduce the PyPI `future` distribution. This rules out the extra name alone as a demonstrated trigger. The report still may expose a metadata- or graph-specific defect, but a reverse dependency chain or complete sanitized reproduction is needed to establish that.

## Related issues and pull requests

- astral-sh/uv#9011 (closed), “Question: What’s the difference between `optional-dependencies` and `dependency-groups` in `pyproject.toml`?” A maintainer demonstrates that a dependency group may contain `package[extra]` to activate that package's published optional dependencies. This supports the generated `private-package[dev,future,test]` representation but does not report an unrelated same-named distribution being installed.
- astral-sh/uv#11937 (closed), “Keeping extras when running `uv sync`.” Maintainer comments explain exact sync and why packages activated only by a removed extra are subsequently removed. It covers the cleanup portion of this report, not the origin of `future`.
- astral-sh/uv#18965 (closed), “uv why # shows why a package is included.” A maintainer identifies `uv tree --invert --package <package>` as the command for tracing a package back to its dependents. `uv tree --invert --package future` is the most direct next diagnostic for this report.

No related pull request was close enough to include. Historical merged fixes for extras parsing were inspected but concern materially different triggers.

## Search coverage and exclusions

Literal searches covered `uv add --extra`, the identifier `future`, PyPI installation, `optional-dependencies`, `dependency-groups`, and `package[extra]`. Conceptual searches covered local, path, and workspace dependencies; same-name package collisions; dependency confusion; unexpectedly installed and transitive dependencies; exact-sync removal; and reverse dependency inspection. Open and closed issues and open, closed, and merged pull requests were searched. Fix-oriented searches covered extra-marker evaluation, invalid extras, optional-dependency resolution, and workspace-extra changes.

The strongest ruled-out candidates were:

- astral-sh/uv#6279 and astral-sh/uv#6324 reported dependencies of unselected extras being installed when the extra names began with `_` or `-`. Merged astral-sh/uv#6395 changed invalid-extra marker evaluation. Those reports involve invalid extra names and unrequested dependencies; `future` is a valid extra explicitly selected in astral-sh/uv#20994, so they do not establish a regression here.
- astral-sh/uv#20151 reports that selecting an extra introduces a transitive package whose configured source is not propagated. Its canonical follow-up is source handling for indirect dependencies, not conversion of an extra name into a package requirement.
- astral-sh/uv#12325 concerns publishing workspace members referenced by extras and does not match installation of an unrelated distribution.

## Evidence needed next

Ask for `uv tree --invert --package future`, which should reveal every path from `future` back to the direct package or group that requires it. If that output is insufficient or private names must be redacted, request the relevant `[[package]]` block for `future`, the referring dependency entries from `uv.lock`, and a complete sanitized workspace reproducer. This evidence will distinguish a transitive requirement in one of the private packages' metadata from an incorrect edge created by uv.
