---
title: Using uv in CircleCI
description: A guide to using uv to install project dependencies and run tests in CircleCI.
---

# Using uv in CircleCI

CircleCI's [Python orb](https://circleci.com/developer/orbs/orb/circleci/python) supports uv as a
package manager. Recent
[`cimg/python` images](https://circleci.com/developer/images/image/cimg/python) also include uv.

## Running tests

The following configuration assumes a uv project with a committed `uv.lock` file and a
[`unittest`](https://docs.python.org/3/library/unittest.html#test-discovery) suite in `tests/`:

```yaml title=".circleci/config.yml"
version: 2.1

orbs:
  python: circleci/python@4.0.0

jobs:
  test:
    executor:
      name: python/default
      tag: "3.12.14"
    environment:
      UV_PYTHON_DOWNLOADS: never
    steps:
      - checkout
      - python/install-packages:
          pkg-manager: uv
          args: --locked
      - run:
          name: Run tests
          command: uv run --locked python -m unittest discover --start-directory tests

workflows:
  test:
    jobs:
      - test
```

Choose an image tag compatible with the project's `requires-python` constraint and `.python-version`
file. Setting `UV_PYTHON_DOWNLOADS=never` ensures the job uses an already-installed Python version.
The `--locked` flag requires the committed lockfile to be up to date.

Replace the `unittest` command with the project's test command as needed. For example, a project
that includes pytest in its development dependencies can use `uv run --locked pytest tests`.

The Python orb also manages dependency caching. See the
[CircleCI caching documentation](https://circleci.com/docs/guides/optimize/caching/#the-uv-package-installer)
and the [uv cache guidance for CI](../../concepts/cache.md#caching-in-continuous-integration) for
details. If the image does not include the required uv version,
[install uv](../../getting-started/installation.md) before running the package-installation step.
