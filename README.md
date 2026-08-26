# Fail on `uv pip install` in case of conflicting versions

Issue: astral-sh/uv#21303

Classification: enhancement

## Summary

The reporter asks `uv pip install` to fail when a later invocation changes a package version in a way that conflicts with a requirement represented by an already-installed package. The report gives the abstract sequence `uv pip install pkg<2` followed by `uv pip install tool>2`, but no concrete packages, versions, uv version, platform, or command output.

Repository evidence establishes that separate `uv pip install` invocations intentionally resolve independently: a constraint passed only to the first command is not automatically a constraint on the second command. This exact successive-install symptom was discussed in astral-sh/uv#18551. Current enforcement is split between `uv pip install --strict`, which reports relevant environment incompatibilities as warnings while returning success, and `uv pip check`, which returns a failure status when it finds incompatibilities.

The requested hard failure is therefore a change to existing install behavior. The report also leaves an important semantic question open: whether failure means returning nonzero after modifying the environment or detecting the conflict before installation and leaving the environment unchanged.

## Draft response

Each `uv pip install` invocation is currently resolved independently, so a constraint supplied only to an earlier invocation is not carried into the next one. `uv pip install --strict` can report relevant incompatibilities after installation, but it still exits successfully; `uv pip check` returns nonzero when the resulting environment is incompatible.

For existing workflows, put the complete constraint set in a `pyproject.toml` or requirements file, or run `uv pip check` as a separate enforcement step. Making `uv pip install` itself fail would be a behavior change. Could you provide concrete package names and versions and clarify whether you need only a nonzero status after installation, or require the environment to remain unchanged when a conflict is detected?

## Classification

Enhancement. Maintainer comments in astral-sh/uv#18551 confirm that each `uv pip install` is independent by design, matching pip's installation model, so the second invocation does not retain an earlier command-line bound as a resolver constraint. The source and astral-sh/uv#10398 establish that `--strict` performs post-install environment diagnostics but reports them as warnings and returns success. By contrast, the separately implemented `uv pip check` returns failure when incompatibilities exist.

Changing `uv pip install` to return failure, and possibly to make the operation atomic or roll it back, would add stronger behavior than the current interface provides. No existing issue or pull request found in the search tracks that exact install failure request. It is not a duplicate of astral-sh/uv#18551 because that issue was closed after explaining independent resolution, and its maintainers treated conflict notification as a distinct concern. It is not a regression: the recent astral-sh/uv#20388 fixed a strict-diagnostic omission but intentionally retained warning-only semantics.

## Related

- astral-sh/uv#18551 — Same successive-install symptom: a second `uv pip install` upgraded `dbt-core` beyond an already-installed package's upper bound. Maintainers confirmed that invocations are independent, recommended declaring the complete constraint set in a `pyproject.toml` or requirements file, and pointed to `--strict` for opt-in diagnostics. The issue is closed and does not track hard-failure or rollback semantics.
- astral-sh/uv#10398 — Documents the relationship between `uv pip install --strict` and `uv pip check`. Maintainers said they share environment diagnostics, with strict install filtering unrelated packages; a later comment records that strict install warns and returns zero while `uv pip check` returns nonzero.
- astral-sh/uv#11055 — Open adjacent request for `uv pip tree --strict` to return failure when it reports conflicts. It demonstrates the same warning-versus-exit-status concern, but targets a read-only tree command rather than install conflict handling or transaction semantics.
- astral-sh/uv#20388 — Merged on 2026-07-14. It makes `uv pip install --strict` report environment diagnostics even when all requested requirements were already satisfied. It improves detection coverage but does not change warnings into installation failure.
- astral-sh/uv#2397 — Merged implementation of `uv pip check`, including success when compatible and failure status when conflicts are present. This is the current explicit enforcement mechanism for an environment assembled through separate install invocations.

## Search evidence

Open and closed issues and open, closed, and merged pull requests were searched with literal terms for `uv pip install`, conflicting versions, silent reinstall or upgrade, `--strict`, and exit status. Conceptual searches covered independent invocations, installed-package constraints, environment validation, incompatible dependencies, downgrade warnings, resolver upgrade strategy, and `uv pip check`. Fix-oriented searches included closed version-specific reports and recent merged strict-diagnostic and check implementations. Candidate comments and their referenced discussions were inspected.

astral-sh/uv#18025 was plausible from its downgrade-warning language but was ruled out because it concerns communicating valid resolver-selected downgrades, not detecting an incompatible installed environment. astral-sh/uv#4779 was also ruled out: it concerns which dependencies `--upgrade` should update, rather than whether an install that leaves conflicts should fail. astral-sh/uv#19223 concerns choosing among valid resolutions and likewise does not match a post-install incompatibility.
