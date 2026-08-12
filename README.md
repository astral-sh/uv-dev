# `uv run` uses wrong python interpreter inside virtualenv with copied python

Issue: astral-sh/uv#21077

Classification: bug

## Summary

The reported failure is reproducible. An activated Python 3.13 virtual environment created with the
standard-library API `venv.create(".venv")` works by itself, but `uv run --with ipython ipython`
selects a differently versioned `python3` from the base installation, then the generated entry point
fails with `ModuleNotFoundError: No module named 'IPython'`. A symlinked environment created by
`python3.13 -m venv .venv` does not switch interpreters and succeeds.

No existing issue or pull request was found that tracks this same combination of an activated,
copied virtual environment and `uv run --with` selecting a different base interpreter. The closest
reports cover copied interpreters, the `--with` ephemeral-overlay design, and stale interpreter
metadata separately.

The reproduction confirms the relevant interpreter-selection path. uv first discovers the copied
environment as Python 3.13, then resolves its `sys._base_executable` to the adjacent unversioned
`python3`, queries that executable as Python 3.12, and uses it for the cached requirements and
temporary overlay environments. This is the same version-layout distinction reported on CentOS,
where the adjacent unversioned interpreter is Python 3.9.

## Reproduction

Outcome: **reproducible** with both the reported uv 0.12.2 and the installed uv 0.12.3 on Linux
x86_64.

All files, Python installations, uv tool installations, and caches were isolated under a new `/tmp`
directory. The fixture used Python 3.13.15 for the active environment and `/usr/bin/python3` 3.12.3
as the default interpreter. To reproduce the CentOS layout without changing the host, a temporary
copy of the Python 3.13 installation retained `bin/python3.13`, while its temporary `bin/python` and
`bin/python3` symlinks targeted `/usr/bin/python3`. The copied environment was then created with the
same standard-library API as the report:

```console
$ python-root/bin/python3.13 -c 'import venv; venv.create("copied/.venv")'
$ copied/.venv/bin/python3 -I -c 'import sys; print(sys.version.split()[0]); print(sys._base_executable)'
3.13.15
/tmp/.../python-root/bin/python3
$ python-root/bin/python3 --version
Python 3.12.3
$ VIRTUAL_ENV=$PWD/copied/.venv PATH=$PWD/copied/.venv/bin:$PATH uv run --with ipython ipython --version
Traceback (most recent call last):
  File "/tmp/.../cache/builds-v0/.tmp.../bin/ipython", line 4, in <module>
    from IPython import start_ipython
ModuleNotFoundError: No module named 'IPython'
```

Verbose uv 0.12.3 output showed that uv found the active environment as CPython 3.13.15, then
assessed `python-root/bin/python3` as the base candidate, cached through that executable, and solved
with installed Python 3.12.3. uv 0.12.2 produced the same traceback. Thus the observed failure is not
inferred from source inspection; it occurred in both tested uv versions.

Two controls succeeded:

- With an ordinary Python 3.13 installation where `python`, `python3`, and `python3.13` all resolve to
  Python 3.13, the copied environment printed IPython 9.16.1.
- With the mixed-version installation but a symlinked environment created by
  `python-root/bin/python3.13 -m venv symlinked/.venv`, `.venv/bin/python3` resolved to `python3.13`
  and the same uv 0.12.2 command printed IPython 9.16.1.

No integration test was found for a standard-library copied environment whose adjacent unversioned
base executable is a different Python version. The closest coverage is
`crates/uv/tests/project/run.rs::run_with_pyvenv_cfg_file`, which verifies `uv run --with` metadata
and parent search paths using a normal uv-created environment; it does not exercise copied
executables or mismatched base-interpreter aliases.

## Draft response

Thanks for the reproducer. We reproduced this with uv 0.12.2 using an active copied Python 3.13
environment whose adjacent `python3` alias is an older system Python. uv initially discovers the
active environment as Python 3.13, but the `--with` path then uses the older alias as the base for
the cached requirements and temporary overlay environment. The resulting IPython entry point fails
with the same `ModuleNotFoundError`. The equivalent symlinked Python 3.13 environment succeeds.

`uv run --with` should preserve the Python version selected by the active environment here, so this
is a bug. A regression test should cover a copied environment with differently versioned
`python3` and `python3.13` executables in the base installation.

## Classification

This is a `bug`, not an enhancement or support question. The environment's interpreter works
directly, and the same `uv run --with` command works when only the environment's executable layout
changes. Installing the requested package for one interpreter while executing the generated command
with another interpreter is incorrect behavior.

It is not a duplicate. Searches across open and closed issues and open, closed, and merged pull
requests found no existing tracker for this exact copied-environment plus `uv run --with` failure.
The local reproduction confirms the mechanism with the same uv version and an equivalent
mixed-version executable layout; the reporter's exact CentOS paths were not required to trigger it.

## Related

- astral-sh/uv#8879 (open issue), “Creating venvs from uv python installtions creates broken venvs” —
  the closest match for the trigger: it explicitly covers both `python -m venv --copies` and
  `venv.create(symlinks=False)`. It differs because a copied uv-managed Python executable cannot
  start due to a missing shared library; in astral-sh/uv#21077 the copied system-Python environment
  runs normally and only the `uv run --with` overlay uses the wrong interpreter.
- astral-sh/uv#12140 (open issue), “Discrepancy in environment creation between projects and
  `uv run`” — documents the same `uv run --with` subsystem and the maintainer-confirmed behavior of
  layering an ephemeral requirements environment over a parent environment. Its issue is differing
  package visibility (`setuptools`), not selection of another Python for a copied environment.
- astral-sh/uv#21066 (open issue), “UV recreates the venv on every run.” — a recent report in which
  virtual-environment interpreter metadata disagreed with the selected base interpreter. Maintainer
  investigation identified stale interpreter-cache data and `--refresh` resolves that report;
  astral-sh/uv#21077 instead has a deterministic copied-versus-symlinked trigger and a `--with`
  import failure.

## Search evidence

Literal searches covered `venv.create`, copied Python/executable, `--copies`, the exact
`ModuleNotFoundError`, `uv run --with`, wrong Python/interpreter, and the copied-versus-symlinked
comparison. Conceptual searches covered active `VIRTUAL_ENV` discovery, virtual-environment base
executables, `sys._base_executable`, cached and ephemeral environments, interpreter caches, overlay
site-packages, symlink resolution, and shebang interpreter selection. Fix-oriented searches applied
the same terms to closed issues and to open, closed, and merged pull requests; no matching pull
request was found.

Several plausible candidates were inspected and ruled out as canonical trackers. astral-sh/uv#19563
has nearly the same headline symptom, but its reporter confirmed that `UV_PYTHON=/usr/bin/python3`
was explicitly forcing the system interpreter. astral-sh/uv#13198 concerns a wrong console-script
shebang created by `uv pip install --prefix`, not `uv run --with` or copied environments.
astral-sh/uv#21075 fixes interpreter-cache identity when `PYTHONEXECUTABLE` or
`__PYVENV_LAUNCHER__` changes launcher context and references astral-sh/uv#21062; neither report uses
the copied-environment trigger in astral-sh/uv#21077.
