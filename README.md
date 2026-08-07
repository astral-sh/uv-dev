# Unable to install `redis[hiredis]` correctly

Issue: astral-sh/uv#20996

Classification: question

## Summary

The reported mixed installation is reproducible, but uv is not resolving redis to version 2.0.0.
The project installs both `redis==8.1.0` and `booktype==1.5`; the latter incorrectly contains a
vendored copy of the `redis` package at version 2.0.0. Both distributions claim paths such as
`redis/__init__.py` and `redis/client.py`, so installing them together creates a file-install race.

This explains the screenshot: the environment has `redis-8.1.0.dist-info` and modern redis modules,
but the displayed `redis/__init__.py` came from booktype's bundled redis 2.0.0. Depending on install
order, redis 8.1.0 or booktype's old files can win individual overlapping paths.

## Reproduction

Outcome: **reproducible** with the installed `uv 0.12.2 (x86_64-unknown-linux-gnu)`, managed CPython
3.12.13, and the exact Tsinghua index URL shown in the report. The reporter used Windows 11 x86-64;
the observed collision was reproduced on Linux and concerns pure Python files shared by the same two
distributions.

A fresh sync of the complete dependency list visible in the screenshot resolved 120 packages and
produced the reported state immediately. The essential conflicting requirements reduce to:

```toml
[project]
name = "redis-reproduction"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = [
    "booktype>=1.5",
    "redis[hiredis]>=8.1.0",
]

[[tool.uv.index]]
url = "https://mirrors.tuna.tsinghua.edu.cn/pypi/web/simple"
default = true
```

The following minimal sequence also produced the collision:

```console
$ uv sync --python 3.12.13 --no-progress
$ uv sync --reinstall --preview-features detect-module-conflicts --no-progress
warning: The file `redis/__init__.py` is provided by more than one package, which causes an install race condition and can result in a broken module. Packages containing the file:
* booktype (booktype-1.5-py3-none-any.whl)
* redis (redis-8.1.0-py3-none-any.whl)
```

After that sync, uv's resolved and installed distribution versions were still redis 8.1.0 and
hiredis 3.4.0, while the import package contained booktype's redis 2.0.0 files:

```console
$ .venv/bin/python -c "from importlib.metadata import version; print(version('redis')); print(version('hiredis'))"
8.1.0
3.4.0
$ rg '^__version__' .venv/lib/python3.12/site-packages/redis/__init__.py
6:__version__ = '2.0.0'
$ .venv/bin/python -c "import redis"
  File ".../site-packages/redis/client.py", line 53
    except socket.error, e:
           ^^^^^^^^^^^^^^^
SyntaxError: multiple exception types must be parenthesized
```

The installed `RECORD` files confirm the collision rather than merely suggesting it:

```text
redis-8.1.0.dist-info/RECORD:redis/__init__.py,...,2929
booktype-1.5.dist-info/RECORD:redis/__init__.py,...,402
```

As controls, clean projects containing only `redis[hiredis]>=8.1.0` installed consistently from
both PyPI and the Tsinghua mirror: distribution metadata and `redis.__version__` both reported
8.1.0, hiredis 3.4.0 was installed, and `uv pip check` passed. A first install of the reduced
two-package fixture also happened to leave redis 8.1.0's file in place; reinstalling both packages
reversed the race and reproduced version 2.0.0.

Existing integration coverage documents this intended warning behavior in
`crates/uv/tests/pip_install/pip_install.rs`, test `overlapping_packages_warning`: overlapping files
are installed without a warning by default, while `--preview-features detect-module-conflicts`
warns that the overlap is an install race which can produce a broken module.

## Classification

This remains classified as a question rather than an uv resolver bug. The reported broken
environment is real, but the selected redis distribution is 8.1.0. The incompatible 2.0.0 source
files are supplied by the independently installed booktype 1.5 wheel, whose `RECORD` overlaps the
redis distribution. uv already has preview-only conflict detection for this condition.

The practical resolution is to remove `booktype` if it is not the intended package, replace it with
a distribution that does not bundle redis, or ask booktype's publisher to remove the vendored
top-level `redis` package. Recreating the environment without booktype produces a consistent redis
8.1.0 installation.

## Draft response

I can reproduce the mixed files, but uv is resolving redis itself to 8.1.0. `booktype==1.5` also
ships a top-level `redis` package containing redis 2.0.0, so both installed distributions overwrite
the same files. That is why the `redis-8.1.0.dist-info` metadata and editor annotation say 8.1.0
while `redis/__init__.py` can say 2.0.0.

Running `uv sync --reinstall --preview-features detect-module-conflicts` reports the collision
between booktype and redis directly. Removing or replacing booktype, or having its publisher stop
bundling the `redis` package, avoids the broken environment.

## Related

- astral-sh/uv#15357 tracks detection of files provided by multiple distributions, the exact class
  of collision reproduced here.
- astral-sh/uv#15253 added the preview `detect-module-conflicts` warning used to confirm this
  reproduction.

## Search and supporting evidence

Repository searches covered redis, hiredis, extras, incomplete installs, and overlapping package
files. No redis-specific regression or resolver test was found. The directly relevant coverage is
`overlapping_packages_warning`, which creates two distributions that own the same module file and
asserts both the default silent behavior and the preview warning. The earlier incomplete-install
reports astral-sh/uv#16468 and astral-sh/uv#16116 are not the same failure: this reproduction has
complete metadata and a confirmed second distribution owning and overwriting the affected files.
