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

The reporter clarified that a submission shim copies the current Python scripts to a per-run
directory and gives Slurm the copied entry-point path, allowing development to continue without
changing the submitted code. Only read-only data is shared. In contrast, every job executes the
same binaries from one shared `.venv`, because copying an approximately 10 GB environment to every
runner is impractical. This rules out the maintainer's example of a shared project checkout with a
distinct home/cache per hardware environment.

The concrete failure sequence is now clear: an NVIDIA job runs `uv sync --extra cuda` and starts
executing from the shared environment; before it finishes, an AMD job runs
`uv sync --extra rocm` against that same environment. The second sync replaces the mutually
exclusive package set while the first process is still using it, and the reporter observes runtime
errors. uv's installer lock serializes sync operations but is not held for the lifetime of the
executing job, so it does not protect a running process from a later sync.

The `centralized-project-envs` preview feature derives an environment from the project path and
interpreter identity, not from the selected accelerator extra. With a different copied project path
for every run, it may create a distinct centralized environment for every copy instead of a reusable
CUDA/ROCm pair. Package files may still be linked from a common cache when the filesystem supports
it, but the reporter's environment reuse and link topology need clarification before treating this
as a suitable workaround.

A maintainer has confirmed two directly applicable manual options because the setup step already
knows whether it selected `--extra cuda` or `--extra rocm`: point each copied checkout's `.venv`
symlink at a hardware-specific shared environment, or select that target with
`UV_PROJECT_ENVIRONMENT`. The maintainer also confirmed that sharing the uv cache and enabling
centralized project environments could let uv manage the symlink, but warned that the current cache
key has too few inputs to rule out collisions between these configurations.

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
should not be used unchanged. A maintainer has since confirmed the hardware-specific symlink and
`UV_PROJECT_ENVIRONMENT` approaches, while warning that centralized project environments can still
clash because their key does not capture all relevant configuration dimensions.

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

- A submission shim copies the current Python scripts into a per-run directory and submits that
  copied entry point to Slurm; project code is not shared between runners.
- Data files are shared read-only and are not implicated in the environment race.
- All runners use the same binaries in one `.venv`, regardless of whether the assigned GPU is
  NVIDIA or AMD; copying the approximately 10 GB environment per runner is not practical.
- The failure occurs while a job is still executing: a second job's sync changes the shared
  environment from CUDA to ROCm or vice versa underneath the first job.

Still needed to evaluate the available workarounds:

- Are the uv cache and home directory shared across runners, and are the cache and environment on a
  filesystem that supports hardlinks or reflinks?
- Are copied project paths stable and reused across jobs, or unique for every run?

These details determine whether centralized project environments would be reusable. If every copied
project path is unique, the project-path-derived key implies a separate environment per run. If the
paths are stable by accelerator, it may be possible to reuse two centralized environments, although
the feature itself still does not select environments based on CUDA/ROCm extras. The maintainer has
therefore identified explicit hardware-specific symlink targets or `UV_PROJECT_ENVIRONMENT` values
as the collision-free options available with the current setup.

## Confirmed workaround options

- During the existing hardware-detection step, link `.venv` to one persistent CUDA environment or
  one persistent ROCm environment before syncing the corresponding extra.
- Alternatively, set `UV_PROJECT_ENVIRONMENT` to a persistent CUDA- or ROCm-specific path during
  that same step. This avoids managing the `.venv` symlink directly.
- A shared uv cache can avoid duplicating cached package content and can be combined with the
  `centralized-project-envs` preview feature, but the maintainer cautions that centralized
  environment keys can clash because they are not keyed by the selected accelerator extra. It is
  therefore not yet established as a safe replacement for two explicit targets in this topology.

## Requested capability

The reporter agrees that separate `.venv-cuda` and `.venv-rocm` targets selected through
`UV_PROJECT_ENVIRONMENT` would address the immediate failure. The remaining request is for uv to
automatically select or manage the appropriate persistent environment from the detected accelerator
or chosen mutually exclusive extra. That automatic accelerator-aware selection is not currently
available and aligns with the broader multi-environment work tracked in astral-sh/uv#20247.

The reporter also notes that the current centralized-project-environment and preview-feature
documentation does not include an example detailed enough for them to evaluate or deploy it in this
Slurm topology. This is a documentation gap reported by the user, not yet a maintainer decision to
expand the documentation.

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
needed to confirm that behavior in this cluster. The latest maintainer guidance confirms that a
shared cache plus centralized environments is plausible but may still clash, whereas selecting two
explicit symlink targets or `UV_PROJECT_ENVIRONMENT` paths is unambiguous.
