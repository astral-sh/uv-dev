# Fail on `uv pip install` in case of conflicting versions

Issue: astral-sh/uv#21303

Classification: enhancement

## Summary

The reporter asks for an opt-in mode in which `uv pip install` remembers explicit requirements from earlier invocations and treats them as constraints on later invocations. Their proposed `--amend` mode would reject a later requirement that contradicts an earlier one, while leaving the current default behavior unchanged. The motivating workflow is a Zephyr project that first installs packages returned by `west packages pip` and later installs more packages into the same virtual environment; the reporter says the later command can change versions required by the earlier command and that the problem is then discovered at runtime.

`west packages pip` does not return a consolidated dependency set. It emits pip arguments containing multiple `-r` requirement-file paths drawn from Zephyr, MCUboot, and workspace modules. The reporter says these files collectively contain hundreds of requirements and constraints, so copying and maintaining them independently in a project `pyproject.toml` is not practical for their workflow.

Repository evidence establishes that separate `uv pip install` invocations intentionally resolve independently: a constraint passed only to the first command is not automatically a constraint on the second command. This exact successive-install symptom was discussed in astral-sh/uv#18551. Current enforcement is split between `uv pip install --strict`, which reports relevant environment incompatibilities as warnings while returning success, and `uv pip check`, which returns a failure status when it finds incompatibilities.

The requested history-aware failure is therefore new stateful behavior rather than stricter validation of the resulting dependency graph. A maintainer recommends using the top-level project interface with a `pyproject.toml` that records the complete constraints and says it is unlikely this behavior will be added to `uv pip` when that interface is available. The reporter's generated, multi-file input explains why that migration is not straightforward, but does not yet establish that the top-level interface cannot model or consume the workflow. The maintainer also asks how the version mismatch causes an actual failure; the issue still provides no concrete conflicting packages, application error, uv version, platform, or runnable Zephyr reproduction.

## Current workaround

The reporter's script stores the arguments produced by `west packages pip`, appends any additional requirements files, and passes the accumulated argument set to every `uv pip install` invocation. This keeps all prior requirements in each resolution and works for scripted installs. It does not protect the environment when a user later runs an independent `uv pip install <package>` command, which is the remaining motivation for persisted constraint history or another guardrail.

## Maintainer direction

A maintainer recommends migrating this workflow from `uv pip` to the top-level uv project interface and recording the relevant constraints in `pyproject.toml`. They offered to help map the workflow to that interface if it does not meet the reporter's needs, but indicated that adding invocation-history semantics to `uv pip` is unlikely because a more suitable interface already exists.

The reporter has now established that the existing inputs are a dynamically generated list of requirement files rather than a dependency list they own. The next interface investigation is whether the top-level project workflow can consume or derive from those generated files without requiring the reporter to duplicate and maintain hundreds of Zephyr requirements and constraints. Separately, a concrete example is still needed to show the runtime breakage caused by a changed version. The latter matters because the abstract example changes the version of the same directly requested package across commands; without persisted requirement history, the installed environment does not itself contain metadata expressing the superseded command-line constraint.

## Classification

Enhancement. Maintainer comments in astral-sh/uv#18551 confirm that each `uv pip install` is independent by design, matching pip's installation model, so the second invocation does not retain an earlier command-line bound as a resolver constraint. The source and astral-sh/uv#10398 establish that `--strict` performs post-install environment diagnostics but reports them as warnings and returns success. By contrast, the separately implemented `uv pip check` returns failure when incompatibilities exist.

Persisting explicit requirements across invocations and resolving later commands against that history would add state and semantics that the current interface does not provide. It is distinct from returning failure for an incompatible installed dependency graph: the proposed example remembers a superseded direct command-line requirement even when no installed distribution metadata records that bound. No existing issue or pull request found in the search tracks that exact history-aware mode. It is not a duplicate of astral-sh/uv#18551 because that issue was closed after explaining independent resolution, and its maintainers treated conflict notification as a distinct concern. It is not a regression: the recent astral-sh/uv#20388 fixed a strict-diagnostic omission but intentionally retained warning-only semantics.

The maintainer's recommendation and expectation lower the likelihood that this will be implemented specifically for `uv pip`, but do not change the classification: the issue still requests a new opt-in capability.

## Related

- astral-sh/uv#18551 — Same successive-install symptom: a second `uv pip install` upgraded `dbt-core` beyond an already-installed package's upper bound. Maintainers confirmed that invocations are independent, recommended declaring the complete constraint set in a `pyproject.toml` or requirements file, and pointed to `--strict` for opt-in diagnostics. The issue is closed and does not track hard-failure or rollback semantics.
- astral-sh/uv#10398 — Documents the relationship between `uv pip install --strict` and `uv pip check`. Maintainers said they share environment diagnostics, with strict install filtering unrelated packages; a later comment records that strict install warns and returns zero while `uv pip check` returns nonzero.
- astral-sh/uv#11055 — Open adjacent request for `uv pip tree --strict` to return failure when it reports conflicts. It demonstrates the same warning-versus-exit-status concern, but targets a read-only tree command rather than install conflict handling or transaction semantics.
- astral-sh/uv#20388 — Merged on 2026-07-14. It makes `uv pip install --strict` report environment diagnostics even when all requested requirements were already satisfied. It improves detection coverage but does not change warnings into installation failure.
- astral-sh/uv#2397 — Merged implementation of `uv pip check`, including success when compatible and failure status when conflicts are present. This is the current explicit enforcement mechanism for an environment assembled through separate install invocations.

## Search evidence

Open and closed issues and open, closed, and merged pull requests were searched with literal terms for `uv pip install`, conflicting versions, silent reinstall or upgrade, `--strict`, and exit status. Conceptual searches covered independent invocations, installed-package constraints, environment validation, incompatible dependencies, downgrade warnings, resolver upgrade strategy, and `uv pip check`. Fix-oriented searches included closed version-specific reports and recent merged strict-diagnostic and check implementations. Candidate comments and their referenced discussions were inspected.

astral-sh/uv#18025 was plausible from its downgrade-warning language but was ruled out because it concerns communicating valid resolver-selected downgrades, not detecting an incompatible installed environment. astral-sh/uv#4779 was also ruled out: it concerns which dependencies `--upgrade` should update, rather than whether an install that leaves conflicts should fail. astral-sh/uv#19223 concerns choosing among valid resolutions and likewise does not match a post-install incompatibility.
