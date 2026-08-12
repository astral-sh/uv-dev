# `uv run` uses wrong python interpreter inside virtualenv with copied python

Issue: astral-sh/uv#21077

Classification: bug

## Summary

The reporter activates a Python 3.13 virtual environment created with the standard-library API
`venv.create(".venv")`, whose POSIX default is to copy the interpreter. Running
`uv run --with ipython ipython` then launches through the system-default Python 3.9 and the generated
entry point fails with `ModuleNotFoundError: No module named 'IPython'`. Recreating the environment
with `python3.13 -m venv .venv`, which uses symlinks on this platform, makes the same command work
with Python 3.13. The report is for uv 0.12.2 on CentOS Stream 9.

No existing issue or pull request was found that tracks this same combination of an activated,
copied virtual environment and `uv run --with` selecting a different base interpreter. The closest
reports cover copied interpreters, the `--with` ephemeral-overlay design, and stale interpreter
metadata separately.

Repository evidence supports treating the behavior as a bug. `uv run --with` creates a cached
requirements environment and then a temporary virtual environment layered over the selected parent.
On Unix, the cached environment resolves a virtual environment back to a base interpreter through
`Interpreter::find_base_python`, while virtual-environment creation normally derives its base from
`sys._base_executable`. A local check with the standard library confirms that a copied environment
can report an unversioned base executable (for example, `/usr/bin/python`) where the equivalent
symlinked environment reports a versioned executable. That is consistent with the reported switch
to the system default, but the exact values on the reporter's CentOS installation remain to be
confirmed with verbose logs and interpreter metadata.

## Draft response

Thanks for the reproducer. `uv run --with` should preserve the Python 3.13 interpreter selected by
the active environment here; using Python 3.9 and then failing to import the requested dependency is
a bug.

The copied-versus-symlinked distinction is relevant because the `--with` path derives a base
interpreter for its cached requirements environment and temporary overlay. A copied standard-library
environment can report a different, unversioned `sys._base_executable`, but we should confirm the
exact paths uv sees on this system before assigning that as the root cause. Could you share the
output of the following from the activated copied environment?

```console
uv run -vv --with ipython ipython
python -I -c 'import sys; print(sys.version); print(sys.executable); print(sys._base_executable); print(sys.prefix); print(sys.base_prefix)'
cat .venv/pyvenv.cfg
```

That should show where interpreter selection changes and give us what we need for a regression test.

## Classification

This is a `bug`, not an enhancement or support question. The environment's interpreter works
directly, and the same `uv run --with` command works when only the environment's executable layout
changes. Installing the requested package for one interpreter while executing the generated command
with another interpreter is incorrect behavior.

It is not a duplicate. Searches across open and closed issues and open, closed, and merged pull
requests found no existing tracker for this exact copied-environment plus `uv run --with` failure.
The source makes the reported mechanism plausible, but verbose output is still needed to confirm the
specific base-executable path on CentOS; that uncertainty does not change the correctness
classification.

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
