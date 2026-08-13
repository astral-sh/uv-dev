# Relative indexes in PEP 723 scripts are resolved against the current working directory

Issue: astral-sh/uv#21096

Classification: bug

## Summary

The reported behavior is reproducible with uv 0.12.3. A file-backed PEP 723 script can install a
dependency from a relative flat index when run from the script's directory, but running that same
script by path from a sibling directory makes uv look for the index under the caller's current
working directory. The script's dependency metadata therefore changes meaning based on the
invocation directory.

The closest precedent is merged astral-sh/uv#10827, which fixed analogous resolution of relative
indexes and find-links paths from `pyproject.toml` and `uv.toml`. Open astral-sh/uv#15055 concerns
lockfile serialization of relative flat indexes, which is distinct from this runtime path-selection
behavior. Merged astral-sh/uv#9208 added PEP 723 index support, but the existing runtime test uses an
HTTPS index and does not exercise relative filesystem paths.

## Reproduction

Outcome: **reproducible**.

Environment used:

- uv 0.12.3 (`x86_64-unknown-linux-gnu`), the same uv release reported in astral-sh/uv#21096
- Linux x86_64; the report used Darwin 25.5.0 arm64
- System Python 3.12.3; the report used Python 3.14.0
- Separate temporary uv caches for each invocation, with every fixture, environment, and cache
  under `/tmp`

The minimal fixture consisted of `scripts/main.py`, a locally generated pure-Python wheel at
`scripts/links/localdemo-1.0.0-py3-none-any.whl`, and an empty sibling directory named `elsewhere`.
The relevant inline metadata was:

```python
# /// script
# requires-python = ">=3.11"
# dependencies = ["localdemo"]
#
# [[tool.uv.index]]
# name = "local"
# url = "./links"
# format = "flat"
#
# [tool.uv.sources]
# localdemo = { index = "local" }
# ///
```

After creating that fixture, the two targeted commands were equivalent to:

```console
$ cd /tmp/uv-21096/scripts
$ UV_CACHE_DIR=/tmp/uv-21096/cache-script uv run --offline --python /usr/bin/python3 main.py
Installed 1 package in 0.43ms
loaded from local flat index

$ cd /tmp/uv-21096/elsewhere
$ UV_CACHE_DIR=/tmp/uv-21096/cache-elsewhere uv run --offline --python /usr/bin/python3 /tmp/uv-21096/scripts/main.py
error: Failed to read `--find-links` directory: /tmp/uv-21096/elsewhere/links
  Caused by: No such file or directory (os error 2)
```

The first command exited 0, installed the generated wheel, and imported it successfully. The second
command exited 2 and named `elsewhere/links` in the error. This directly observes that `./links` was
resolved from the process working directory rather than from the directory containing `main.py`.

Existing integration tests do not cover this runtime combination:

- `crates/uv/tests/project/run.rs`, `run_pep723_script_index`, verifies that a PEP 723 script can
  resolve through a named HTTPS index, but it uses no relative path and runs `main.py` from its
  containing directory.
- `crates/uv/tests/lock/lock.rs`, `lock_find_links_relative_url`, verifies a relative flat index in
  `pyproject.toml`, not inline script metadata.
- `crates/uv/tests/project/edit.rs`, `add_index_with_existing_relative_path_in_script`, verifies
  rewriting an index path while editing a script outside the working directory with `--frozen`; it
  does not resolve or run a dependency from that inline index.

Source inspection is consistent with the observation but is not needed to infer it:
`Pep723ItemRef::directory` computes a file-backed script's containing directory, and dependency
lowering receives that directory for relative requirements and sources, while
`Pep723ItemRef::indexes` returns the inline index definitions as parsed. The targeted command above,
rather than this source-level asymmetry, is the evidence for the reproduced behavior.

## Fix

Outcome: **fixed** in the checkout.

The parent regression test at `crates/uv/tests/project/run.rs`,
`run_pep723_script_relative_flat_index`, was first run unchanged and passed while asserting the
undesirable exit code 2 and `elsewhere/links` lookup. Its snapshot was then changed to require a
successful installation of `ok==1.0.0`; before the production change, that desired assertion failed
with the reported `Failed to read --find-links directory: [TEMP_DIR]/elsewhere/links` error.

The production fix has two deliberately scoped parts:

- File-backed PEP 723 `FilesystemOptions` are rebased against the script's directory before they
  are combined with discovered configuration. This gives the resolver and installer a stable local
  index location while leaving stdin and remote-script behavior unchanged.
- The inline indexes passed to non-workspace requirement and extra-build-dependency lowering are
  cloned and rebased against `Pep723ItemRef::directory`. This ensures a named source records the
  same script-relative index URL instead of retaining the caller's working-directory interpretation.

Successful focused debug-profile validation:

```console
$ cargo test --package uv --test project run_pep723_script_relative_flat_index
test run::run_pep723_script_relative_flat_index ... ok

$ cargo test --package uv --test project run_pep723_script_index
test run::run_pep723_script_index ... ok

$ cargo test --package uv --test project run_pep723_script_no_sources_package
test run::run_pep723_script_no_sources_package ... ok

$ cargo fmt --all --check
```

The nearby HTTPS-index test and package-scoped source-disabling test confirm that existing PEP 723
index behavior remains intact. Formatting and `git diff --check` also pass. Only the parent test and
the affected production files are modified.

## Draft response

Thanks for the clear reproduction. I reproduced this with uv 0.12.3 on Linux: the script succeeds
from its own directory, but when invoked by path from a sibling directory uv tries to read that
sibling's `links` directory and exits with the reported `--find-links` error. Existing PEP 723 index
coverage uses an HTTPS URL, while existing relative flat-index coverage uses `pyproject.toml`, so
this specific runtime case is not covered.

## Classification

This is a correctness bug rather than a feature request. A relative index embedded in a file-backed
script should be stable when the same script is invoked from another directory. The observed error
also matches uv's established handling for file-backed `pyproject.toml` and `uv.toml` configuration,
whose relative index and find-links paths are based on the containing configuration file.

This is not a duplicate of the closest related reports. astral-sh/uv#10827 addressed filesystem
configuration rather than PEP 723 metadata, and astral-sh/uv#15055 tracks portable lockfile
representation rather than choosing the runtime base directory. The original PEP 723 index work in
astral-sh/uv#9208 did not test a local relative index.

## Related

- astral-sh/uv#10827 (merged pull request) — Fixed relative index and find-links paths from
  `pyproject.toml` and `uv.toml` by resolving them against the containing configuration file; its
  scope did not include PEP 723 script metadata.
- astral-sh/uv#15055 (open issue) — Concerns an absolute path being stored in `uv.lock` for a
  relative flat index, not current-working-directory resolution while running a script.
- astral-sh/uv#9208 (merged pull request) — Added `[[tool.uv.index]]` support in PEP 723 scripts;
  its runtime test uses `https://test.pypi.org/simple`.

## Search coverage

The checkout was searched for PEP 723 script indexes, relative flat indexes, `--find-links` errors,
and script index path lowering. The three tests listed in the reproduction section are the closest
coverage; none combines a file-backed PEP 723 script, a relative flat index, and invocation from a
different working directory.

Pull request: https://github.com/astral-sh/uv-dev/pull/705
