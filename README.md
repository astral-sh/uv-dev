# Does a uv script access the Internet every time it runs?

Issue: astral-sh/uv#22388

Classification: question

## Summary

The reporter uses `#!/usr/bin/env -S uv run --script` and briefly sees `Resolving dependencies` on every invocation. They ask whether dependencies are resolved over the Internet every time and whether commands defined in `[project.scripts]` behave the same way. They do not report an execution failure or measured network traffic.

The reported environment is Ubuntu 26.04, Linux 7.0.0-38-generic x86_64, with uv 0.12.21 (7af826859, September 29, 2026). The reporter has supplied the PEP 723 metadata: Python >=3.12, six dependencies, and a Git source for simpleaudio pinned to a full commit. They report that removing simpleaudio and its source entry stops the recurring progress message. The executable script body, selected Python version, other configuration, lockfile status, and verbose/network traces remain unavailable.

Behavioral checks with the installed uv 0.12.13 confirm that the progress message does not imply Internet access: it appeared while creating a script lockfile entirely from cache, and while preparing a new script environment with `--offline`. Repeated runs of an unchanged minimal script, both unlocked and locked, and a project entry point reused their environments without Internet connection attempts or the reported progress message. These examples answer the general network question but do not explain this reporter's repeated message. They exercised a registry dependency, not the newly supplied Git source. The reproduction outcome remains **needs_more_information**; the reporter's dependency-removal comparison has not been independently tested.

## Classification

Keep this classified as a question. Neither the report nor the experiments establish incorrect dependency behavior, unnecessary network requests, or a regression. Actual Internet access on every invocation is the reporter's question, not an observed fact in the report.

The supplied metadata now provides a concrete Git-source reproduction lead. The report establishes a user-observed association between that dependency and progress output, not recurring network requests or a confirmed cause. The available binary is older than the reported release, and the existing registry-only fixture does not test this source configuration. No root cause is confirmed. The reporter wants downloads when needed without forcing every invocation offline; whether this requires a behavior change remains unresolved.

astral-sh/uv#7538 is a close historical symptom match, but it predates persistent local script environments. astral-sh/uv#9688 requested script locking, which was implemented before the reported release. Neither history establishes a duplicate or a regression.

## Maintainer guidance and pending information

In astral-sh/uv#22388, maintainer woodruffw explains that invoking a local script through `uv run --script` does not itself require Internet access. Resolving declared dependencies that are not cached can require network access. The maintainer also identifies a separate reason for networking: uv may download a Python version requested by the script metadata when that version is not already available locally, then reuse it on subsequent runs. They recommend `--offline` to disable uv's network access.

The reporter has answered the metadata portion of the maintainer's request. They do not want to put `--offline` in the shebang: fetching missing dependencies is acceptable, but they expect no repeated network access once everything needed is local. Offline mode remains a diagnostic control, not their accepted workflow. The maintainer's general guidance does not establish why the pinned Git dependency is associated with recurring progress. Keep the distinction between cached artifacts and metadata freshness, and between uv's networking and the Python application's networking.

## Reporter-provided metadata and comparison

The supplied header is:

```python
#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "argcomplete>=3,<4",
#     "docker>=7.2.0",
#     "pyyaml>=6,<7",
#     "simpleaudio",
#     "tabulate[widechars]>=0.10.0",
#     "types-docker>=7.2.0.20260827",
# ]
#
# [tool.uv.sources]
# simpleaudio = {
#     git = "https://github.com/cexen/py-simple-audio.git",
#     rev = "6a7cb95c5af4537bad72bad9b190e09cb6d7883c",
# }
# ///
```

The reporter says that deleting both the `simpleaudio` dependency and its `[tool.uv.sources]` entry prevents `Resolving dependencies` from appearing. No executable body, verbose output, request trace, or timing comparison accompanies the header. The revision is a full commit identifier rather than a moving branch. This narrows investigation to the Git-source path and environment-satisfaction checks, but does not establish that uv fetches that repository on each invocation. The other dependencies should not be assumed necessary for a minimal reproduction until the comparison is tested.

## Reproduction

**Outcome: needs_more_information.** The repeated progress message was not observed on the earlier warmed, unchanged registry-only fixtures. The newly supplied Git-source configuration has not been tested. The conceptual question could be explored: displaying `Resolving dependencies` does not require Internet access, and neither script nor project entry-point execution inherently accesses the Internet every time.

### Environment and isolation

- Installed executable on `PATH`: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`; `uv --version` reports `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- Ubuntu 24.04.5 LTS, Linux 6.17.0-1022-azure x86_64, CPython 3.12.3 at `/usr/bin/python3`.
- All fixtures, caches, configuration/data directories, logs, and environments are under `/tmp/uv-22388.T3qdCc`. The project's `.venv` and `uv.lock` are also inside that directory.
- Each process used a constructed environment rather than inherited uv or authentication settings: `UV_NO_CONFIG=1`, `UV_PYTHON=/usr/bin/python3`, `UV_PYTHON_DOWNLOADS=never`, `UV_DEFAULT_INDEX=https://pypi.org/simple`, `UV_KEYRING_PROVIDER=disabled`, temporary `UV_CACHE_DIR`, `UV_CREDENTIALS_DIR`, `UV_PYTHON_INSTALL_DIR`, `TMPDIR`, and XDG paths, and an empty temporary `NETRC`. No existing credential store was inspected or used.
- Commands ran in a pseudo-terminal to capture transient progress. `strace -f -e trace=connect` followed uv and child processes without tracing network payloads. Internet connection counts refer to `AF_INET`/`AF_INET6` calls; remaining warm-run calls were failed local `AF_UNIX` attempts to `/var/run/nscd/socket`.
- This is a targeted reproduction using the installed executable, not a build or execution of the checkout's Rust tests. The reported uv 0.12.21 and Ubuntu 26.04 combination was not tested.

### Script fixture and commands

`example.py` contains:

```python
#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["iniconfig==2.0.0"]
# ///
from importlib.metadata import version
print("iniconfig", version("iniconfig"))
```

With the isolated environment above, these commands were run explicitly in the temporary directory. Each uv/script command was wrapped in the connection trace and terminal capture:

```sh
chmod +x example.py
./example.py
./example.py
uv run -v --script example.py
uv lock --script example.py
./example.py
./example.py
uv run --offline --script example.py
uv run -v --script example.py
```

A second script, `cached.py`, was then created with identical contents but no adjacent lockfile. The changed path gives it a new script environment while retaining the package cache:

```sh
uv run --offline --script cached.py
```

All commands succeeded. Executed scripts printed `iniconfig 2.0.0`.

| Scenario | `Resolving dependencies` in terminal capture | Internet `connect` calls | Observed behavior |
| --- | --- | --- | --- |
| First unlocked shebang run, empty cache | Yes | 20, including two TCP port 443 connections | Fetched and installed the dependency |
| Second unlocked shebang run | No | 0 | Reused the script environment |
| Unlocked verbose run | No | 0 | Logged `All requirements satisfied: iniconfig==2.0.0` |
| `uv lock --script example.py`, cache populated | Yes | 0 (no connection calls of any family) | Created the script lockfile from cached data |
| Two consecutive locked shebang runs | No on either run | 0 on either run | Reused the locked script environment |
| Locked offline run | No | 0 | Succeeded using locally available state |
| Locked verbose run | No | 0 | Logged `Existing uv.lock satisfies workspace requirements` and the installed requirement |
| New script environment, `uv run --offline --script cached.py` | Yes | 0 | Resolved and installed from cached packages while offline |

The last scenario directly demonstrates resolver progress during offline script execution. It does not reproduce the reporter's repeated-message observation for an unchanged script.

### Project entry point

The independent project fixture has no workspace members or dependency groups. Its `pyproject.toml` is:

```toml
[project]
name = "demo"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = ["iniconfig==2.0.0"]

[project.scripts]
demo = "demo:main"

[build-system]
requires = ["uv_build==0.12.13"]
build-backend = "uv_build"
```

`src/demo/__init__.py` defines:

```python
from importlib.metadata import version

def main():
    print("iniconfig", version("iniconfig"))
```

The project shared the already populated temporary dependency cache. Explicit invocations from its root were:

```sh
uv run demo
uv run demo
uv run -v demo
uv run --offline demo
uv run --no-sync demo
./.venv/bin/demo
```

Every invocation succeeded, printed `iniconfig 2.0.0`, and had zero Internet connection attempts. The initial run created the project environment and lockfile and displayed `Resolving dependencies` using cached package data. The second run did not display it. The verbose run logged `Existing uv.lock satisfies workspace requirements`, reported both requirements already installed, and printed the `Resolved 2 packages` / `Checked 2 packages` summaries without Internet connections. The offline, no-sync, and direct-entry-point runs also succeeded. This fixture has no application-level network activity.

### Existing test coverage inspected

- `crates/uv/tests/project/run.rs`, `run_pep723_script`: the setup includes a project requiring `anyio` and a PEP 723 script requiring `iniconfig`. The first script run installs `iniconfig`; the second snapshot has no stderr, and the test asserts that neither creates a script lockfile. This covers persistent script-environment reuse, not a network trace or the exact shebang invocation.
- `crates/uv/tests/project/run.rs`, `run_pep723_script_lock`: explicitly creates a script lockfile, installs its dependency, then runs with `--locked`. That snapshot succeeds with `Resolved 1 package` / `Checked 1 package`, without reinstalling. Later assertions exercise metadata changes and `--locked` / `--frozen`. It does not count network requests.
- `crates/uv/tests/project/run.rs`, `run_script_module_conflict`: defines and installs a `[project.scripts]` entry point using Hatchling. Subsequent invocations, including after adding a competing `__main__.py`, continue executing the entry point and show environment checks rather than reinstalling. It covers entry-point execution/reuse, not absence of networking.
- `crates/uv-client/tests/it/cached_client.rs`, `revalidation_updates_only_policy`: a mock server first serves metadata with `max-age=0`, then returns a 304 response with a fresh policy. A newly constructed client reads that persisted fresh entry with `CacheControl::None` and `AllowStale`; the test explicitly asserts that the server received no requests for those reads. This is HTTP-cache coverage, not the script execution path.

These tests were read, not executed. `TestContext::run` in `crates/uv-test/src/lib.rs` sets `UV_SHOW_RESOLUTION=1`, explaining why integration snapshots contain resolution/check summaries that ordinary successful warm invocations may omit. The summaries and transient progress are not network counters.

### Missing information and limits

The inline metadata and Python version constraint are now available above. Remaining useful information includes whether an adjacent script lockfile exists, the selected Python interpreter, any additional uv configuration, a minimal executable body if needed, and sanitized verbose output from consecutive executions on the reported uv 0.12.21. Network observations should distinguish actual connections from resolver progress. For the entry-point question, request the relevant `[project]`, `[project.scripts]`, build-system, source, workspace, and dependency-group configuration only if its actual behavior also needs investigation.

Automatic Python downloads were not tested: the fixtures selected an existing `/usr/bin/python3` and set `UV_PYTHON_DOWNLOADS=never`. The missing-interpreter download path identified by the maintainer therefore remains outside these reproduction findings; the reporter has supplied `requires-python = ">=3.12"`, but not the selected interpreter. No delayed metadata-expiration or custom-index/source scenario was tested. In particular, the newly reported Git source was absent from the earlier fixtures, so those results do not rule out behavior specific to this dependency. Connection tracing is not a full packet capture. No recurring Internet access or root cause for recurring progress is established. Offline mode limits uv's networking, not the Python application's networking.

Reproduction artifacts are retained under `/tmp/uv-22388.T3qdCc`: `reproduce.py`, `project_checks.py`, the fixtures, `results.jsonl`, and per-command `.terminal.log` / `.connect.log` files. No checkout files or GitHub content were changed; pre-existing checkout modifications were left untouched.

## Follow-up investigation

Use the supplied metadata as the starting point for a focused comparison on uv 0.12.21: consecutive warm runs with the pinned Git dependency, then the reporter's variant removing both the dependency and its source entry. Separate environment-satisfaction checks, resolution, Git operations, metadata reads, builds, and actual network connections in the evidence. A smaller fixture containing only the pinned dependency can then determine whether the other requirements matter. These are investigation steps, not completed experiments.

Establish lockfile status before drawing conclusions about locked versus unlocked execution. Offline execution can distinguish cache sufficiency from online behavior, but it does not satisfy the reporter's requested workflow by itself. If unnecessary repeated network access is established, reassess the classification and search for the specific Git-source behavior rather than general cache-policy requests. The project-entry-point question remains separate; no corresponding project reproduction has been supplied.

## Related

- astral-sh/uv#7538 (issue, closed) — "uv run" runs resolve too much. Reports the same shebang-script progress message and slower runs after a pause. Maintainers explained cached resolution and PyPI's metadata expiration. This describes older script behavior; persistent local script environments subsequently changed the execution path.
- astral-sh/uv#11347 (pull request, merged) — Use a stable directory for (local) script virtual environments. Merged February 12, 2025; replaced resolution-keyed environments for local scripts with persistent environments and update_environment. The reported 0.12.21 source includes the resulting shortcut when installed requirements are satisfied, so historical always-resolve explanations cannot be applied unchanged.
- astral-sh/uv#9688 (issue, closed) — Feat: Create a cached lock file for `uv run` with scripts. Follow-up to astral-sh/uv#7538 explicitly requested avoiding repeated resolution and network dependence. Closed by script-lockfile support in astral-sh/uv#10136. The new report asks for clarification and does not establish failure of that implemented capability.
- astral-sh/uv#10135 (pull request, merged) — Add support for locking PEP 723 scripts. Merged January 8, 2025; implemented uv lock --script and closed astral-sh/uv#6318. Establishes the available option for recording script dependency versions; locking alone is not an offline guarantee.
- astral-sh/uv#10136 (pull request, merged) — Respect PEP 723 script lockfiles in uv run. Merged January 8, 2025; made uv run reuse an existing script lockfile and closed astral-sh/uv#9688. Supports the proposed script-locking guidance and predates the reported version.
- astral-sh/uv#6091 (pull request, merged) — Validate lockfile (rather than re-resolve) in `uv lock`. Project-side background: replaced a second resolution with validation of an existing lockfile. Explains why project lock checks need not resolve dependencies afresh. Its original lockfile-instability bug, astral-sh/uv#6063, is not reported here.

## Supporting evidence

- **Progress is independent of HTTP requests.** `crates/uv-resolve-operations/src/reporters.rs:34` sets `Resolving dependencies...` when constructing the resolver reporter. The observed offline run and cache-only lock creation corroborate that this message can appear without Internet access.
- **Satisfied script environments can skip resolution.** `crates/uv-environment-operations/src/lib.rs`, `update_environment`, checks installed requirements and returns early in sufficient mode when no reinstall/upgrade or source-tree processing is required. The warm unlocked fixture logged that its requirements were satisfied. This is conditional behavior, not a promise for all configurations.
- **The reported release has the relevant support.** Source references at 0.12.21 remain useful context: `crates/uv/src/commands/project/run.rs` for script environments, `crates/uv/src/commands/project/mod.rs` for `update_environment`, and `crates/uv/src/commands/reporters.rs` for progress. Source and historical issue discussion do not substitute for testing the reported binary.
- **Caching and offline mode differ.** `docs/concepts/cache.md` documents dependency caching; `docs/concepts/indexes.md` describes HTTP cache-control, including PyPI's ten-minute metadata cache. If metadata is needed and is missing or expired, requests may be necessary. Expiration does not by itself force an already satisfied script environment to resolve again. These expiration scenarios were not exercised here.
- **Locking is explicit for scripts.** `docs/guides/scripts.md` documents `uv lock --script example.py` and adjacent lockfile reuse. The relevant locking and persistent-environment changes in Related predate the reported release.
- **Project entry points use the project workflow.** `docs/concepts/projects/config.md` describes `[project.scripts]`; `docs/concepts/projects/sync.md` explains automatic lock/sync checks, `--locked`, `--frozen`, and `--no-sync`. New registry releases alone do not invalidate a project lockfile.

## Search scope and exclusions

Distinct questions covered: (1) repeated resolver progress for a shebang-invoked local script; (2) whether this implies repeated Internet access and what caching/offline/locking controls apply; and (3) project entry-point execution through uv versus direct invocation. Version and platform were treated as conditions, not assumed causes.

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs. Separately searched the script message/shebang and project.scripts questions using Resolving dependencies, script/internet/network, uv run/every time, project.scripts/offline/network, then cache, re-resolve, lockfile, offline fallback, area:cache, area:scripts, and version 0.12.21. Supplemented empty PR search responses with an all-state PR listing (limit 20,000), filtering script environments, caching, resolution, locking, offline, and no-sync titles. Inspected candidate bodies, comments, review comments, and linked issue/PR chains; checked source at 0.12.21. Ruled out astral-sh/uv#12903 (pinned uv tool run), astral-sh/uv#15454 and astral-sh/uv#10380 (requests for cache-first/offline fallback), and astral-sh/uv#15156 (case-sensitive imports on a case-insensitive filesystem). astral-sh/uv#10123 was an unmerged draft superseded by the listed locking PRs.

The linked discussion chain was astral-sh/uv#7538 → astral-sh/uv#9688 and astral-sh/uv#6318 → astral-sh/uv#10135 / astral-sh/uv#10136. The abandoned draft astral-sh/uv#10123 was closed in favor of other PRs. astral-sh/uv#11472 contains a maintainer explanation that script environments use stable cached paths and are reused. astral-sh/uv#6091 led to astral-sh/uv#6063, whose lockfile-instability symptoms are absent here.

The especially plausible tool-network report astral-sh/uv#12903 concerns explicitly pinned `uv tool run` packages and a maintainer-confirmed expectation about avoiding requests. The reporter here does not use that command or supply equivalent evidence. astral-sh/uv#15454 and astral-sh/uv#10380 request additional cache/fallback policies. The reporter's clarified preference for fetching only missing dependencies now makes them adjacent workflow discussions, but neither establishes the same pinned-Git-source behavior; the earlier exclusion should not be read as saying the reporter has no cache-policy preference. astral-sh/uv#15156 concerns module-name casing during installation, not resolver progress or repeated networking.

