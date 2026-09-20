# uv version modifies pyproject.toml on disk even when lock_and_sync fails (e.g., under --locked)

Issue: astral-sh/uv#21854

Classification: bug

## Summary

`uv version --bump minor --locked` updates `pyproject.toml` from `0.1.0` to `0.2.0`, then correctly rejects the now-outdated `uv.lock`, but leaves the edited `pyproject.toml` on disk. The same behavior occurs when locked mode is enabled through `UV_LOCKED=1`. This leaves project metadata and the lockfile inconsistent even though the command reported failure.

The behavior was reproduced with the installed uv executable. Current source is consistent with the observation: `project_version` calls `update_project`, which sets the version and writes `pyproject.toml`, before calling `lock_and_sync`. Locked validation then runs through `LockOperation` with `LockMode::Locked`; its error is propagated without restoring the original file. By comparison, the project-editing flow in `add.rs` snapshots the target and lockfile and reverts them when locking or syncing fails.

No existing issue or open pull request was found for this same rollback failure. The closest history establishes that `uv version` is intended to keep the project file and lockfile synchronized and that a static version change is expected to invalidate a locked lockfile. Neither point makes the partial on-disk edit correct.

## Reproduction

Outcome: reproducible.

The targeted reproduction used uv 0.12.13 (`x86_64-unknown-linux-gnu`) on Linux x86_64 with CPython 3.12.3. The report names uv 0.12.17 on Windows 11 x86_64 and Linux x86_64; only the installed uv 0.12.13 executable was available for this check. All project files, the uv cache, and the managed-Python directory were placed under a fresh `/tmp` directory. Locked environment variables were unset during setup so that the initial lockfile could be created.

```console
$ repro_root=$(mktemp -d /tmp/uv-21854.XXXXXX)
$ mkdir "$repro_root/project" && cd "$repro_root/project"
$ export UV_CACHE_DIR="$repro_root/cache"
$ export UV_PYTHON_INSTALL_DIR="$repro_root/python"
$ unset UV_LOCKED UV_FROZEN
$ uv init --name demo
$ uv lock
$ uv version --bump minor --locked
error: The lockfile at `uv.lock` needs to be updated, but `--locked` was provided.

hint: To update the lockfile, run `uv lock`.
```

The final command exited with status 2. Before it ran, both `pyproject.toml` and the `demo` entry in `uv.lock` had version `0.1.0`. After the error, the SHA-256 digest of `pyproject.toml` had changed and its version was `0.2.0`; the digest of `uv.lock` was unchanged and its `demo` entry remained at `0.1.0`. This directly reproduces the reported partial on-disk mutation with an existing lockfile.

The environment-variable variant was also reproduced from a separately initialized and locked temporary project:

```console
$ UV_LOCKED=1 uv version --bump minor
error: The lockfile at `uv.lock` needs to be updated, but `UV_LOCKED=1` was provided.
```

It also exited with status 2, changed `pyproject.toml` to `0.2.0`, and left `uv.lock` unchanged at `0.1.0`.

Existing integration coverage in `crates/uv/tests/it/version.rs` does not assert rollback for this failure. `version_bump_minor` covers a successful bump, while `version_set_workspace` covers locked reads and a locked no-op set whose version already matches the lockfile. Neither test performs a version-changing locked operation that fails and then checks both files.

## Draft response

Thanks, this is reproducible. The `--locked` error is expected because changing a static project version makes the lockfile out of date, but `uv version` should not retain its `pyproject.toml` edit after that validation fails. The current implementation writes the file before `lock_and_sync` and propagates the error without restoring it.

astral-sh/uv#13254 and astral-sh/uv#13317 explain why the command updates the lockfile, but they do not cover rollback on failure. The next step is an integration regression test asserting that both `pyproject.toml` and `uv.lock` remain unchanged after `uv version --bump minor --locked`, followed by making the version edit use the same snapshot-and-revert behavior as other project-editing commands.

## Classification

This is a bug, not an enhancement or question. Locked validation is behaving as designed: maintainers confirmed in astral-sh/uv#15643 that changing a static project version makes the lockfile out of date. The correctness problem is that the failing command retains its own partial mutation.

The mechanism is source-confirmed rather than inferred from the report: `update_project` writes `pyproject.toml` before `lock_and_sync`, and the error path has no rollback. No open issue or pull request already centralizes this problem, so the issue is not a duplicate.

## Related

- astral-sh/uv#13254 — “uv version leaves lock file out-of-date” (closed). This is the historical inverse of the new failure: updating only `pyproject.toml` left `uv.lock` stale. It established that `uv version` should keep both files synchronized.
- astral-sh/uv#13317 — “make `uv version` lock and sync” (merged). This implemented the current flow and explicitly described `--locked` as erroring when a requested version change makes the lockfile stale. Its implementation writes the project file before validation and does not include the snapshot-and-revert behavior used by `uv add`.
- astral-sh/uv#15643 — “uv sync --locked --no-install-project fails if only project version changes” (open). Maintainer discussion confirms that a version-only metadata change should fail locked validation. Unlike astral-sh/uv#21854, it starts from a file changed outside the failing command and does not report a missing rollback.

## Search and evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal searches used `uv version`, `--locked`, `UV_LOCKED`, `pyproject.toml`, `lock_and_sync`, and the exact lockfile-update error. Conceptual searches covered transactional edits, rollback/revert, failed commands leaving metadata changed, version-bump lock/sync semantics, and historical fixes.

astral-sh/uv#15286 was the strongest plausible candidate that was ruled out. It asks why version bumps perform a full lock and sync and requests a way to avoid or reduce dependency resolution; it does not report or track restoration of files after a failed operation.
