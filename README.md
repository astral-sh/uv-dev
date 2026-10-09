# Separate runtime workspace ownership from cache cleanup

Issue: astral-sh/uv#22338

Classification: bug

## Summary

The reported loss of a live PEP 517 backend's environment is reproducible. Independently reconstructed, dependency-free fixtures on Linux x86_64 with uv 0.12.13 and CPython 3.12.3 demonstrated both reported failure modes:

- Killing only uv with `SIGKILL`, then running plain `uv cache clean`, removed the surviving backend's environment. Its subsequent write failed with `FileNotFoundError`, errno 2 (`ENOENT`), and a delayed import from that environment failed with `ModuleNotFoundError`.
- Installing two local projects concurrently, with one backend intentionally failing after the other became ready, left the second backend alive after uv exited. Its environment had disappeared, and the same write and import failed. This scenario sent no signals and ran no cache-maintenance command.

The prune case is version-specific: the installed uv 0.12.13 retained the live environment after parent-only termination, and both probes succeeded. The report's prune result used a newer PR revision associated with astral-sh/uv#22171; that binary was not executed here. The macOS versions in the report were also not rerun. These limits do not negate the two independently observed failures.

Separating execution workspaces from evictable cache entries remains a proposed design. These experiments establish the observable lifetime problem, not a particular implementation or a complete solution for descendants.

## Reported conditions and requested behavior

- **Parent termination and cleanup:** macOS 26.6.2 arm64, Python 3.14.7, uv 0.12.23; parent-only `SIGKILL`, followed by plain `uv cache clean`, while the backend remains alive.
- **Pruning:** CI revision `013216c409a2ce74821f25d9d84ce0747a0301ba` of astral-sh/uv#22171. This is a PR revision, not a claimed released-version prune reproduction.
- **Parallel cancellation:** one build fails while another backend still needs its environment. The report does not provide its exact command or fixtures; the independent reproduction below uses `uv pip install` for two local projects.
- **Expected behavior:** surviving backends retain usable execution files and imports after uv dies or another build fails. Retaining a detached descendant's workspace after its direct parent exits and its output pipes close is an additional requested acceptance criterion, not a separately demonstrated result here.
- **Architecture proposal:** move execution sources, temporary environments, and reusable executable environments outside `UV_CACHE_DIR`; manage process shutdown, output collection, and workspace lifetime together; avoid runtime symlinks into evictable cache entries; create reusable environments at their final paths; conservatively retain entries when process lifetime is uncertain. Automatic crash recovery remains outside the proposal's scope.

No last-known-good version is supplied for the clean or parallel-cancellation cases. Their reproduction on 0.12.13 shows that these behaviors are not limited to the reported 0.12.23 release; it does not identify their first affected release.

## Classification

Retain the bug classification for the live-backend lifetime problem, with architectural design work tracked separately. `uv cache clean` is documented to remove all cache entries; reproducing removal alone would not establish a defect. The additional evidence is that a still-executing backend loses an environment it was successfully using, including in a parallel-build failure without any user-requested cleanup. Whether and how uv guarantees retention after uncatchable parent termination remains a design decision.

The evidence does not establish that moving all runtime storage outside the cache is the only acceptable fix. It also does not establish recurrence of the abandoned-directory leak fixed by astral-sh/uv#22171: that issue concerned retaining directories after their processes stopped, while this issue concerns deleting directories whose users remain alive.

## Reproduction

**Outcome: reproducible**, for parent-only termination followed by plain cache clean and for sibling-build failure. The reported newer prune behavior and detached-descendant criterion remain unvalidated at runtime.

### Environment and isolation

- Installed executable from `PATH`: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`.
- Version: `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- Platform: Linux x86_64; interpreter: `/usr/bin/python3`, CPython 3.12.3.
- Inspected checkout: `01b62808962d7abfe2d10f43d652f357d8038202`, whose uv manifest reports 0.13.0. The checkout was inspected, not built or executed.
- Original reproduction root: `/tmp/uv-22338-repro.SDKgTZ`. Each scenario had its own cache, source projects, control files, output directory, and `TMPDIR`. Commands ran with an explicit environment, offline mode, disabled configuration discovery, and Python downloads disabled. No package downloads or external build backends were needed.
- Retained artifacts beside this README: `reproduction/reproduce.py`, the three `*-report.json` files, and the corresponding `*-uv.log` files. The script is the exact harness executed for these observations.

### Minimal fixtures and commands

Each local project uses an in-tree PEP 517 backend and no build requirements. The slow project's `pyproject.toml` is:

```toml
[project]
name = "slow_build"
version = "0.1.0"

[build-system]
requires = []
build-backend = "backend"
backend-path = ["."]
```

The backend's `get_requires_for_build_wheel` returns `[]`. Its metadata hook writes static metadata so dependency resolution need not run a wheel build. In `build_wheel`, it successfully writes `before.txt` under `sys.prefix` and creates a tiny module, `lifetime_probe_dependency.py`, in that environment's `site-packages`. It then records its PID and `sys.prefix` outside the cache and waits for an external release file. On release, it tries `(Path(sys.prefix) / "after.txt").write_text(...)` and a first import of that module, records the results outside the cache, and exits. The module contains only `VALUE = 42`; this tests delayed access to environment files without downloading dependencies. The probe deliberately stops after measurement instead of producing a wheel. A 30-second backend deadline bounds waiting.

For the parallel case, a second otherwise equivalent project named `fail_build` raises `RuntimeError("Intentional sibling build failure")` in `build_wheel` only after the slow backend has signaled readiness. This handshake ensures that the failing build overlaps a live sibling. These are ordinary local projects, with no workspace, dependency groups, workspace sources, or frozen execution involved.

The following commands rerun the retained harness with the installed `uv` on `PATH`; each invocation creates its own named scenario directory under the fresh reproduction directory:

```sh
repro_dir=$(mktemp -d /tmp/uv-22338-rerun.XXXXXX)
cp "$RUNNER_TEMP/issue-context/reproduction/reproduce.py" "$repro_dir/reproduce.py"
python3 "$repro_dir/reproduce.py" clean
python3 "$repro_dir/reproduce.py" parallel
python3 "$repro_dir/reproduce.py" prune
```

The harness executes these core command sequences, with paths scoped to each scenario:

```sh
# Shared command environment, set explicitly by the harness:
# UV_CACHE_DIR=<scenario>/cache
# TMPDIR=<scenario>/tmp
# UV_PYTHON_INSTALL_DIR=<scenario>/python
# UV_PYTHON_DOWNLOADS=never UV_NO_CONFIG=1 UV_OFFLINE=1
# UV_CONCURRENT_BUILDS=2 UV_NO_PROGRESS=1
# PROBE_CONTROL=<scenario>/control

# Parent-only termination and clean:
uv build --wheel --python /usr/bin/python3 --out-dir <scenario>/dist <scenario>/slow_build
# The harness starts uv asynchronously and waits for the backend's ready marker.
UV_LOCK_TIMEOUT=1 uv cache clean
# The preceding control times out while uv is alive; no --force is used.
kill -KILL <uv-pid>
# Wait for that uv process to exit, then:
uv cache clean
# Release the backend and read its recorded write/import results.

# Sibling-build failure, using a separate cache and source fixtures:
uv venv --python /usr/bin/python3 <scenario>/venv
uv pip install --python <scenario>/venv/bin/python <scenario>/slow_build <scenario>/fail_build
# Wait for uv to exit, release the surviving backend, and read its results.
# No signals or cache-maintenance commands are issued in this scenario.

# Prune comparison, using a separate cache and slow-build fixture:
uv build --wheel --python /usr/bin/python3 --out-dir <scenario>/dist <scenario>/slow_build
# Wait for backend readiness, kill only uv, and wait for uv to exit.
kill -KILL <uv-pid>
uv cache prune
# Release the backend and read its results.
```

### Observed results

| Scenario | uv / cleanup result | Surviving backend's environment | Delayed write and import |
| --- | --- | --- | --- |
| Plain clean while uv is alive, control | Exit 2 after the configured one-second lock timeout; reports cache in use | Still present | Backend remains paused until the subsequent parent-death test |
| Parent-only `SIGKILL`, then plain clean | Parent exit `-9`; clean exit 0, `Removed 29 files (104.0KiB)` | Present after parent death, missing after clean | `FileNotFoundError`, errno 2, and `ModuleNotFoundError` |
| Sibling build raises an error | `uv pip install` exits 1 with the intentional `backend.build_wheel` failure | Missing while slow backend remains alive | `FileNotFoundError`, errno 2, and `ModuleNotFoundError` |
| Parent-only `SIGKILL`, then prune on uv 0.12.13 | Parent exit `-9`; prune exit 0, `No unused entries found` | Still present | Write succeeds; imported value is 42 |

For both failures, the backend PID was still present after uv exited and the same backend subsequently wrote its result file outside the cache. Thus this is demonstrated execution after environment removal, not an inference from a directory listing or a PID check alone. Both environments were under `<cache>/builds-v0/.tmp...`. The parallel scenario did not run the live-lock control, cache clean, or cache prune.

### Existing test coverage inspected

Searches prioritized `crates/uv/tests/it/` and `crates/uv-client/tests/it/`, then followed this checkout's build and pip-install integration-test layout. The following setup and assertions were read:

- `crates/uv/tests/build/cache_clean.rs::cache_timeout` holds an exclusive cache lock, runs clean with `UV_LOCK_TIMEOUT=1`, and snapshots the timeout error. This covers active-lock protection, not a backend surviving uv.
- `crates/uv/tests/build/cache_clean.rs::clean_all` installs packages, then snapshots successful full-cache removal. `clean_force` additionally tests cleaning with an exclusive lock held when `--force` is supplied. Neither exercises orphaned backend access; the reproduced failure requires no `--force`.
- `crates/uv/tests/build/cache_prune.rs::prune_temporary_build_environment` creates `builds-v0/.tmp123456/pyvenv.cfg` manually, snapshots its pruning, and asserts that the directory is gone. It covers an abandoned directory, with no live process.
- `crates/uv/tests/build/build.rs::build_all_with_failure` defines a workspace with two successful builds and one failing setuptools build, runs `uv build --all --no-build-logs`, and asserts that the successful packages' distributions exist. It covers completion of the other workspace builds, not the early-failure behavior of parallel preparation in `uv pip install` or subsequent access by an orphaned backend.

No matching live-backend retention test was found in the inspected areas. No repository tests were added or executed; these were standalone runtime experiments using the installed binary.

### Validation limits

The reporter's macOS 26.6.2 arm64 / Python 3.14.7 / uv 0.12.23 combination, the PR CI prune revision, Windows behavior, and detached descendants with closed output pipes were not run. The 0.12.13 prune success must not be presented as disproof of the newer prune report. The original reporter's fixtures would still help compare implementation details, but they are no longer essential to demonstrate the clean and parallel-build failures. No fix, release bisect, or proof of the proposed ownership architecture is claimed.

## Related

- astral-sh/uv#22171 (pull request, merged) — Prune temporary build environments in `uv cache prune`. Merged on 2026-10-07 to resolve astral-sh/uv#22167. The final implementation removes every builds-v0 entry without checking child-process lifetime, extending the reported exposure to prune. Its test covers an abandoned directory, not a surviving backend.
- astral-sh/uv#22167 (issue, closed) — uv build leaves a temporary build environment after Ctrl+C. The precursor reports leftover build environments after both uv and its backend stop. Closed by astral-sh/uv#22171. The new issue concerns deleting environments while their users remain alive, so it is not a duplicate of this cleanup request.
- astral-sh/uv#11694 (issue, closed) — uv cache prune removes files from a running process executed with `uvx`. Historical report of the same harmful outcome: pruning a live tool environment breaks subsequent imports. A maintainer closed it in November 2025 because uv now held a cache lock. The new parent-death and build-cancellation cases expose a lifetime gap beyond that protection.
- astral-sh/uv#15990 (pull request, merged) — Retain the cache lock and temporary caches during `uv run` and `uvx`. The September 2025 fix retains cache resources throughout command execution, following the shared-lock introduction in astral-sh/uv#15888. It explains the protection behind closing astral-sh/uv#11694, but does not provide ownership that survives uv termination or canceled build futures.
- astral-sh/uv#20116 (pull request, open) — Add automatic pruning of unreferenced cache archives. Proposes releasing the global lock while protecting running environments and their cache symlinks with per-entry locks. Its description explicitly releases claims on process death; it does not establish retention for surviving descendants or cancellation of PEP 517 builds. Related design work, not a confirmed fix or duplicate.
- astral-sh/uv#19321 (issue, open) — Fine-grained cache locking. Canonical discussion for reducing cache-lock contention, linked by maintainers from astral-sh/uv#19317. Comments examine long-lived commands, external environments, cache-backed environments, and symlink dependencies. It addresses maintenance while uv remains alive, rather than the new ownership failures after termination or cancellation.

Additional context retained from the prior issue investigation:

- astral-sh/uv#18962 concerns orphan cleanup and global virtual-environment management; it is related storage-lifecycle work rather than a demonstrated fix for a surviving PEP 517 backend.
- astral-sh/uv#22387 fixes deletion of the cache lock file during concurrent cleanup. Preserving the lock file does not by itself retain execution files after their owning uv process dies; it should not be treated as a duplicate of these observed lifetime failures.
- astral-sh/uv#15888 introduced shared/exclusive cache locking, followed by astral-sh/uv#15990 retaining those resources during command execution. These protect the uv lifetime; neither demonstrates retention after parent-only `SIGKILL`.
- astral-sh/uv#19317 points to the canonical fine-grained locking discussion in astral-sh/uv#19321. The closed autoprune proposal astral-sh/uv#17211 leads to astral-sh/uv#20116. The age-based pruning prototype astral-sh/uv#21374 changes eviction policy while retaining the main lock, without establishing descendant ownership.
- astral-sh/uv#12830, astral-sh/uv#13017, and astral-sh/uv#3095 provide signal-forwarding and process-launch context. The signal-forwarding fix excludes parent-only `SIGKILL`, so it does not establish a fixed-then-regressed guarantee here. astral-sh/uv#12003 concerns external symlink installations rather than this build-lifetime failure.

The issue/PR statuses and duplicate-search conclusions above are preserved from the existing handoff. This runtime pass did not repeat the GitHub search or make any changes on GitHub.

## Supporting source evidence

Source inspected at checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`; it is newer than the executed uv 0.12.13 binary:

- `crates/uv-cache/src/lib.rs:336`: `Cache::venv_dir` and `Cache::build_dir` allocate `TempDir` instances inside `builds-v0`. The runtime probes independently observed environments at these cache-relative paths.
- `crates/uv-cache/src/lib.rs:531`, `crates/uv/src/commands/cache_clean.rs:33`, and `crates/uv/src/commands/cache_prune.rs:31`: shared/exclusive cache locking coordinates uv operations. The clean control demonstrated blocking while uv was alive and successful removal after only that parent was killed.
- `crates/uv-cache/src/lib.rs:583` clears the cache contents. The newer prune implementation at line 750 iterates over all build-bucket entries and removes them without a child-liveness check. This is consistent with the report about astral-sh/uv#22171, but source inspection is not a runtime reproduction of the newer prune behavior.
- `crates/uv-build-frontend/src/lib.rs:246` stores the temporary environment directory in `SourceBuild`. `PythonRunner::run_script` spawns a Tokio child at line 1301, drains its output, and waits for the direct child. No explicit cancellation shutdown owner or `kill_on_drop` configuration appears in the inspected path.
- `crates/uv-installer/src/preparer.rs:64` uses `FuturesUnordered` for parallel preparation; `Preparer::prepare` uses `try_collect` at line 100. Dropping pending build futures after a sibling error is a source-backed explanation consistent with the observed pip-install result. The experiment establishes live-child access after removal; it did not instrument destructors or prove the exact internal cancellation path in uv 0.12.13.

The current changelog and version metadata were inspected, but neither an older clean/parallel last-known-good release nor a runtime comparison with the reported PR binary was established. The existing handoff identifies astral-sh/uv#22171 as merged on 2026-10-07; that historical change is relevant specifically to the newer prune behavior.

## Next steps

Use the retained fixtures as the basis for separate regression scenarios for parent-only termination plus clean and sibling-build failure. Exercise prune with a binary containing astral-sh/uv#22171, and add a distinct descendant-lifetime scenario. Assert delayed file access or imports as well as directory existence. Before implementation, decide when workspace ownership can safely end, how uncertain descendant lifetimes are handled, and how existing cache-resident execution entries are retained during migration.

All reproduction files and caches were confined to temporary directories. The only handoff changes are this README and retained reproduction artifacts under the runner's temporary issue-context directory; no checkout files or GitHub state were modified.
