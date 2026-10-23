---
title: Using uv with marimo
description:
  Use uv with marimo notebooks for interactive computing, Python scripts, and data applications.
---

# Using uv with marimo

[marimo](https://github.com/marimo-team/marimo) is an open-source, interactive Python notebook.
marimo stores notebooks as Python scripts. You can track these scripts with Git, run them directly,
and share them as applications. Because marimo notebooks are Python scripts, they work well with uv.

Use marimo as a standalone tool, with self-contained scripts, in projects, or in other environments.

## Using marimo as a standalone tool

To use a marimo notebook without a project, start a marimo server in an isolated environment:

```console
$ uvx marimo edit
```

Start a specific notebook with:

```console
$ uvx marimo edit my_notebook.py
```

## Using marimo with inline script metadata

marimo notebooks can declare their own dependencies with inline script metadata. See the
[script guide](../../guides/scripts.md) for more information. To add `numpy` as a notebook
dependency, run:

```console
$ uv add --script my_notebook.py numpy
```

To edit a notebook that contains inline script metadata, run:

```console
$ uvx marimo edit --sandbox my_notebook.py
```

marimo uses uv to start the notebook in an isolated virtual environment with its declared
dependencies. If you install packages from the marimo interface, marimo adds them to the script
metadata.

To run a notebook as a Python script without an interactive session, run:

```console
$ uv run my_notebook.py
```

## Using marimo within a project

If marimo is a dependency of your [project](../../concepts/projects/index.md), start a notebook with
access to the project environment:

```console
$ uv run marimo edit my_notebook.py
```

To make more packages available, add them to the project with `uv add`. You can also use the marimo
package installation interface, which runs `uv add` for you.

If marimo is not a project dependency, run the notebook with:

```console
$ uv run --with marimo marimo edit my_notebook.py
```

You can import project modules while you edit the notebook. However, packages that you install
through the marimo interface are not added to the project. These packages might not be available the
next time you run marimo.

## Using marimo in a non-project environment

To run marimo in a virtual environment without a [project](../../concepts/projects/index.md),
install marimo directly in the environment:

```console
$ uv venv
$ uv pip install numpy
$ uv pip install marimo
$ uv run marimo edit
```

The notebook can now use `import numpy`. When you install a package from the marimo interface,
marimo adds it to the environment with `uv pip install`.

## Running marimo notebooks as scripts

You can run a marimo notebook as a script with:

```console
$ uv run my_notebook.py
```

This command runs the notebook as a Python script without an interactive browser session. It works
with inline script metadata, project dependencies, or a non-project environment.
