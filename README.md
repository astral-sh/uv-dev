# Best way to handle architecture dependent environments

Issue: astral-sh/uv#21670

Classification: duplicate

## Summary

The reporter uses mutually exclusive `cuda` and `rocm` extras and conditional package indexes for
PyTorch and JAX on a hybrid GPU Slurm cluster. Each queued job currently runs `uv sync` against the
same `.venv`, so CUDA and ROCm jobs request different final package sets in one shared environment.
They ask whether uv can maintain multiple environments for one project or redirect packages at
runtime according to the available accelerator.

astral-sh/uv#20060 is the closest existing report: it describes Slurm jobs on nodes with different
architectures, shared-filesystem races, and conditionally selecting `UV_PROJECT_ENVIRONMENT`.
astral-sh/uv#15672 covers the general request for two incompatible environments under one project,
and a maintainer confirms that uv does not manage such environments automatically. The supported
workaround is to give each configuration a distinct `UV_PROJECT_ENVIRONMENT` path, as demonstrated
by a maintainer in astral-sh/uv#9906.

The reporter clarified that the project code is not shared: each run receives a separate copy of
the Python files. Only read-only data is shared. The `.venv` is linked rather than copied because
each environment is roughly 10 GB. This rules out the maintainer's example of a shared project
checkout with a distinct home/cache per hardware environment.

The `centralized-project-envs` preview feature derives an environment from the project path and
interpreter identity, not from the selected accelerator extra. With a different copied project path
for every run, it may create a distinct centralized environment for every copy instead of a reusable
CUDA/ROCm pair. Package files may still be linked from a common cache when the filesystem supports
it, but the reporter's environment reuse and link topology need clarification before treating this
as a suitable workaround.

## Draft response

This is the same underlying mixed-environment use case discussed in astral-sh/uv#20060 and, more
generally, astral-sh/uv#15672. uv does not currently manage multiple named project environments
automatically, and a single environment cannot simultaneously hold the mutually exclusive CUDA and
ROCm sync results.

For now, select a distinct project environment path together with each extra, for example
`UV_PROJECT_ENVIRONMENT=.venv-cuda uv sync --extra cuda` and
`UV_PROJECT_ENVIRONMENT=.venv-rocm uv sync --extra rocm`. Keep the same variable set when invoking
`uv run`, or invoke the corresponding environment's Python directly. Pre-creating both environments
before submitting jobs avoids changing them during job startup. Concurrent uv operations are
protected by an environment lock, as documented by astral-sh/uv#2818, but separate paths are still
needed because the two sync commands request different final package sets.

Maintainer handoff note: this draft predates the reporter's storage-topology clarification and
should not be used unchanged. The code checkout is copied per run, so centralized project
environments may be keyed separately for every copied path rather than yielding one reusable
environment per accelerator.

## Classification

This is a duplicate because its underlying need is already tracked closely: astral-sh/uv#20060
covers mixed-architecture Slurm nodes and separate environment paths, while astral-sh/uv#15672 asks
how one project can retain two incompatible environments. The CUDA-versus-ROCm condition is a more
specific instance of that existing multi-environment request.

No incorrect uv behavior is established. `uv sync` reconciles the selected project environment to
the requested extras, so alternating mutually exclusive extras against the same path necessarily
changes that environment. uv serializes concurrent installers with a file-based environment lock,
but serialization cannot make one path retain two different desired package sets. Automatic named
or dependency-set-specific project environments remain unsupported; a virtual environment also does
not dynamically redirect installed packages based on GPU hardware.

## Deployment topology and open questions

Confirmed by the reporter:

- Every run receives its own copy of the Python scripts; project code is not shared between runners.
- Data files are shared read-only and are not implicated in the environment race.
- `.venv` is linked because copying an approximately 10 GB environment to every runner is not
  practical.

Still needed to evaluate the available workarounds:

- What does each runner's `.venv` link point to, and do CUDA and ROCm runners currently share that
  same target?
- Are the uv cache and home directory shared across runners, and are the cache and environment on a
  filesystem that supports hardlinks or reflinks?
- Are copied project paths stable and reused across jobs, or unique for every run?

These details determine whether centralized project environments would be reusable. If every copied
project path is unique, the project-path-derived key implies a separate environment per run. If the
paths are stable by accelerator, it may be possible to reuse two centralized environments, although
the feature itself still does not select environments based on CUDA/ROCm extras.

## Related

- astral-sh/uv#20060 — **Expose `UV_PROJECT_ENVIRONMENT` as flag to `uv sync`** (open issue). This is
  the closest scenario match: Slurm jobs run on nodes with different architectures, shared
  environments can race or become incompatible, and the current workaround is to select distinct
  paths conditionally with `UV_PROJECT_ENVIRONMENT`.
- astral-sh/uv#15672 — **How to manage two uv environments under one project?** (open issue). This
  asks the same core multi-environment question. A uv maintainer states that automatic management of
  two project environments is not supported.
- astral-sh/uv#9906 — **How to handle named environments with different python versions?** (open
  issue). Its trigger is different, but a maintainer gives the directly applicable workaround:
  assign a different `UV_PROJECT_ENVIRONMENT` value to each `uv sync` configuration.
- astral-sh/uv#20247 — **Feature: Named, shared environments independent of a project directory**
  (open issue). This broader enhancement explicitly includes several environments for one project
  when dependency sets cannot coexist; it does not yet provide the requested automatic selection.
- astral-sh/uv#2818 — **Document that uv is safe to run concurrently** (merged pull request). This
  documents that uv locks a target virtual environment during installation. That prevents concurrent
  uv modifications, but it does not make one environment simultaneously represent both the CUDA and
  ROCm sync states.

## Search evidence

Literal searches covered the issue title; multiple `.venv` and project environments;
`UV_PROJECT_ENVIRONMENT`; `triton-rocm` and JAX/ROCm identifiers; hybrid GPU and Slurm wording; and
concurrent or parallel `uv sync`. Conceptual searches covered named environments, dependency
universes, accelerator-dependent dependencies, mutually exclusive extras, architecture-specific
environments, conditional indexes, and HPC workflows. Fix-oriented searches included closed issues
and merged pull requests for concurrent installation and centralized environment storage.

astral-sh/uv#18844 and astral-sh/uv#11418 concern ROCm index resolution and incompatible wheel
selection, not concurrent selection of two valid environments. The centralized storage work in
astral-sh/uv#1495 and astral-sh/uv#18214 does not key environments by GPU accelerator or selected
extras. However, as the maintainer noted, separate home/cache roots for the different hardware
environments would provide physical separation when a project path is shared. The reporter has now
clarified that project paths are copied per run instead, so the path-derived centralized environment
key may prevent cross-run reuse; the remaining cache, link-target, and path-stability details are
needed to confirm that behavior in this cluster.
