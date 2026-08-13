---
title: Using uv with pre-commit
description:
  Use uv with pre-commit to update lockfiles, export requirements, and compile requirements files.
---

# Using uv in pre-commit

The [`astral-sh/uv-pre-commit`](https://github.com/astral-sh/uv-pre-commit) repository provides the
official pre-commit hook.

To use uv with pre-commit, add an example to the `repos` list in `.pre-commit-config.yaml`.

Use `uv-lock` to update `uv.lock` when `pyproject.toml` changes:

```yaml title=".pre-commit-config.yaml"
repos:
  - repo: https://github.com/astral-sh/uv-pre-commit
    # uv version.
    rev: 0.12.10
    hooks:
      - id: uv-lock
```

Use `uv-export` to keep `requirements.txt` synchronized with `uv.lock`:

```yaml title=".pre-commit-config.yaml"
repos:
  - repo: https://github.com/astral-sh/uv-pre-commit
    # uv version.
    rev: 0.12.10
    hooks:
      - id: uv-export
```

Use `pip-compile` to compile requirements files:

```yaml title=".pre-commit-config.yaml"
repos:
  - repo: https://github.com/astral-sh/uv-pre-commit
    # uv version.
    rev: 0.12.10
    hooks:
      # Compile requirements
      - id: pip-compile
        args: [requirements.in, -o, requirements.txt]
```

To compile other requirements files, change `args` and `files`:

```yaml title=".pre-commit-config.yaml"
repos:
  - repo: https://github.com/astral-sh/uv-pre-commit
    # uv version.
    rev: 0.12.10
    hooks:
      # Compile requirements
      - id: pip-compile
        args: [requirements-dev.in, -o, requirements-dev.txt]
        files: ^requirements-dev\.(in|txt)$
```

To run the hook on multiple files, add more entries:

```yaml title=".pre-commit-config.yaml"
repos:
  - repo: https://github.com/astral-sh/uv-pre-commit
    # uv version.
    rev: 0.12.10
    hooks:
      # Compile requirements
      - id: pip-compile
        name: pip-compile requirements.in
        args: [requirements.in, -o, requirements.txt]
      - id: pip-compile
        name: pip-compile requirements-dev.in
        args: [requirements-dev.in, -o, requirements-dev.txt]
        files: ^requirements-dev\.(in|txt)$
```
