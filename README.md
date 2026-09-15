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

A maintainer has now asked how the project code is shared between the hardware environments. If
each hardware environment has a distinct user home (and therefore a distinct uv cache) while the
project checkout is shared, the `centralized-project-envs` preview feature may already isolate the
physical project environments. Whether that applies cannot be determined until the reporter
clarifies the cluster's home, cache, and project-filesystem layout.

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

Maintainer handoff note: this draft predates the storage-topology follow-up and should not be used
unchanged. If the reporter confirms that each hardware environment has a separate home/cache, the
centralized project environment preview may remove the need for explicit `.venv-cuda` and
`.venv-rocm` paths.

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

## Open investigation question

The effectiveness of centralized project environments depends on the cluster's storage topology:

- Are the home directory and uv cache shared by CUDA and ROCm nodes, or unique to each hardware
  environment?
- Is only the project checkout shared through a separate filesystem?

If the home/cache roots are unique while the project path is shared, the preview feature suggested
by the maintainer may produce separate physical environments despite using the same project path.
If the cache is shared and the interpreter identity is also the same, centralized storage does not
by itself distinguish CUDA from ROCm or selected extras, so explicit environment paths remain the
applicable workaround.

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
environments would provide physical separation even when the shared project path and generated
environment key are otherwise the same.
