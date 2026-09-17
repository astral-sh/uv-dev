# Is it possible to use uv to manage dependencies for the Python scripts inside a skill?

Issue: astral-sh/uv#21772

Classification: question

## Summary

The reporter asks whether a skill such as `ppt-master` can use uv instead of pip and a
`requirements.txt` file to manage its Python scripts' dependencies and reduce dependency
conflicts. They do not provide a failing uv command, platform, version, error, or link to the
skill's implementation.

Repository member `zsol` confirmed that the answer is yes and directed the reporter to uv's
[scripts guide](https://docs.astral.sh/uv/guides/scripts/). This maintainer response agrees with
the reproduction and implementation evidence below.

uv already provides the relevant building blocks. PEP 723 inline metadata can give each script
its own declared dependency set; `uv add --script` edits those declarations; and `uv run` creates
and reuses the environment needed to run the script. A script lockfile is optional and can be
created with `uv lock --script`. If all scripts intentionally share one dependency set, the skill
directory can instead be a normal uv project with `pyproject.toml` and `uv.lock`.

The remaining integration requirement is outside uv's automatic control: the skill must invoke
the scripts through `uv run`, and uv must be available in the execution environment. Replacing a
requirements file alone will not change a launcher that continues to call pip or plain Python.

## Reproduction

Outcome: `reproducible`. The capability asked about in astral-sh/uv#21772 works in a minimal
standalone-script fixture.

Environment:

- uv 0.12.13 (`x86_64-unknown-linux-gnu`)
- CPython 3.12.3 at `/usr/bin/python3`
- Ubuntu Linux, x86-64
- All fixture files, caches, managed environments, and lockfiles were placed under a fresh `/tmp`
  directory. Ambient `UV_LOCKED=1` and `UV_FROZEN` settings were disabled because they were not
  part of the report; leaving `UV_LOCKED=1` enabled correctly rejects `uv add` before a lockfile
  exists and is not evidence about the reported question.

Starting with this `script.py`:

```python
import iniconfig

print(f"script dependency imported from {iniconfig.__file__}")
```

the targeted commands were:

```console
$ uv add --script script.py 'iniconfig==2.1.0'
Resolved 1 package in 138ms
$ uv run script.py
Installed 1 package in 2ms
script dependency imported from /tmp/.../cache/environments-v2/script-.../lib/python3.12/site-packages/iniconfig/__init__.py
$ uv run script.py
script dependency imported from /tmp/.../cache/environments-v2/script-.../lib/python3.12/site-packages/iniconfig/__init__.py
$ uv lock --script script.py
Resolved 1 package in 2ms
$ uv run --locked script.py
script dependency imported from /tmp/.../cache/environments-v2/script-.../lib/python3.12/site-packages/iniconfig/__init__.py
```

`uv add` inserted a PEP 723 block containing `iniconfig==2.1.0`. The first `uv run` installed the
package into a script-specific cached environment, and the second reused the same environment with
no installation output. `uv lock --script` created `script.py.lock`, after which `uv run --locked`
succeeded.

The shared-project alternative was also observed with a minimal `pyproject.toml` declaring
`dependencies = ["iniconfig"]`. `uv run --project project project/tool.py` created `project/uv.lock`
and imported the dependency from `project/.venv`, separate from the script-specific cached
environment.

Existing integration coverage matches these observations:

- `crates/uv/tests/project/edit.rs`, `add_script`: verifies `uv add --script` adds a dependency to
  inline script metadata and does not implicitly create a lockfile.
- `crates/uv/tests/project/run.rs`, `run_pep723_script`: verifies that `uv run` installs inline
  dependencies into a script environment and reuses that environment on the next invocation.
- `crates/uv/tests/project/run.rs`, `run_pep723_script_lock`: verifies explicit script locking,
  successful execution with the lockfile, and successful `uv run --locked`.

This confirms that uv can manage and isolate dependencies for Python scripts shipped in a skill.
It does not make skill runtimes automatically recognize uv metadata: the skill must invoke the
script with `uv run`, and uv must be installed in that runtime. The report has no `ppt-master`
repository or launcher commands, so compatibility with that particular skill was not tested.

## Draft response

Yes, provided uv is available where the skill runs and the skill invokes its scripts through uv.
For per-script isolation, add PEP 723 metadata with
`uv add --script path/to/script.py <dependencies>` and run the script with
`uv run path/to/script.py`; uv will create and reuse the script's own environment instead of
installing those packages into a shared environment. If reproducible resolutions are needed, ship
the adjacent lockfile created by `uv lock --script path/to/script.py`.

If every script in the skill intentionally shares one dependency set, the skill directory can
instead be a normal uv project with `pyproject.toml` and `uv.lock`, with scripts launched via
`uv run`.

This requires changing the skill's install/run instructions or launcher. Merely replacing
`requirements.txt` will not affect a launcher that still calls pip or plain Python. If ppt-master
cannot be changed that way, please provide its repository and the exact install and run commands so
we can identify the integration constraint.

## Classification

This is a `question`. The report asks whether an existing workflow is possible and does not
establish incorrect uv behavior. Existing script support already covers dependency declaration,
execution in a script-specific environment, cached reuse, and optional locking. No prior issue
specifically tracks integration with `ppt-master` or another skill, so there is no canonical issue
to which astral-sh/uv#21772 should be closed as a duplicate. The report also does not request a
specific new uv capability.

Official OpenAI documentation describes a skill as a directory that may contain scripts, but does
not define a Python dependency installation mechanism for those scripts. That is consistent with
treating uv usage as part of the skill's own execution instructions rather than as automatic skill
manifest behavior.

## Related

- astral-sh/uv#3096 — **Add PEP 723 support to `uv run`** (closed). This is the canonical request
  for executing scripts with inline dependency metadata and establishes the execution model that
  can be used by a skill.
- astral-sh/uv#4656 — **Add PEP 723 support to uv run** (merged pull request). This implemented
  `uv run foo.py` for PEP 723 scripts and creates the environment needed for the declared
  dependencies.
- astral-sh/uv#4667 — **Support PEP 723 scripts in `uv add`** (closed). This is the canonical
  request for managing dependencies directly in an individual script with `uv add --script`.
- astral-sh/uv#5995 — **Support PEP 723 scripts in `uv add` and `uv remove`** (merged pull
  request). This implemented dependency addition and removal for scripts, including creating an
  inline metadata block when one is absent.
- astral-sh/uv#10445 — **Automatic Inline Dependency Management with uv add and uv sync**
  (closed). This closely related support request was answered by pointing to `uv add --script`;
  its additional request to mirror a project's dependencies automatically was not adopted.
- astral-sh/uv#6318 — **Make it possible to lock dependencies in a script** (closed). This is the
  canonical discussion for reproducible per-script dependency resolutions.
- astral-sh/uv#10135 — **Add support for locking PEP 723 scripts** (merged pull request). This
  implemented `uv lock --script`, which can create an adjacent lockfile for a skill script.

## Supporting evidence

Repository member `zsol` answered the question affirmatively and recommended the scripts guide in
the discussion on astral-sh/uv#21772. This establishes the maintainer-supported direction without
adding any new requirement or limitation specific to `ppt-master`.

The current scripts guide recommends a project or inline metadata for declaring dependencies. It
documents `uv add --script`, says `uv run` automatically creates an environment with the script's
dependencies, and notes that inline-script execution ignores project dependencies. It also
documents `uv lock --script` and reuse of the resulting lockfile by later script operations.

The implementation history confirms the complete capability chain: astral-sh/uv#4656 added PEP
723 execution, astral-sh/uv#5995 added dependency editing, and astral-sh/uv#10135 added locking.
The merged environment work in astral-sh/uv#4789 and astral-sh/uv#11347 further confirms that
script environments are cached and use stable per-script paths, but those pull requests are
supporting implementation details rather than the closest user-facing discussions.

## Search scope

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `ppt-master`, `Codex skill dependencies`, `skill requirements.txt`, `Python skill`,
`manage dependencies`, `script dependencies`, and `dependency conflicts`. Conceptual and
fix-oriented queries included `PEP 723`, `inline script metadata`, `isolated environment`,
`cached environment`, `standalone scripts`, `uv run`, `uv add --script`, and script locking.

The strongest candidates and their comments and referenced fixes were inspected through the chains
astral-sh/uv#3096 to astral-sh/uv#4656, astral-sh/uv#4667 to astral-sh/uv#5995, and
astral-sh/uv#6318 to astral-sh/uv#10135; the related support discussion in astral-sh/uv#10445 was
also inspected. astral-sh/uv#11383 was plausible but is not the canonical match because it
requests a different capability: mapping centrally declared `pyproject.toml` groups to individual
scripts rather than using existing inline metadata. astral-sh/uv#8856 was also inspected and ruled
out because it concerns creating an IDE-visible local environment, not running skill scripts in
uv-managed isolation.
