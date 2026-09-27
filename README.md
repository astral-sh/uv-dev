# uv tool install --force --reinstall mutates the tool environment in place: concurrent readers observe missing files for tens of seconds while the install exits 0

Issue: astral-sh/uv#22006

Classification: bug

## Summary

The reported reader-visible interruption is reproducible. With an isolated tool directory and a controlled local wheel, an external process repeatedly importing from the installed tool environment observed a failed import during `uv tool install --force --reinstall`, while uv 0.12.13 completed successfully. A faster observer targeting a module installed late in the wheel saw 34 failures over about 0.87 seconds during another successful reinstall.

No existing issue specifically tracks atomic publication of installed tool environments. The closest historical work is astral-sh/uv#5503 and astral-sh/uv#5509, which made cached environments safe to recreate while in use by constructing them elsewhere and publishing them through a symlink. astral-sh/uv#13883 tracks stronger locking between uv operations, but external processes already running from a tool environment cannot participate in uv's locks.

## Reproduction

Outcome: **reproducible** on Linux x86_64 with uv 0.12.13 and Python 3.12.3. The report used macOS 15.7.3 arm64, uv 0.12.6, and Python 3.13.15, so the behavior also persists in a newer uv version on a different platform.

The reproduction used only directories under `$RUNNER_TEMP`. A synthetic `issue22006-tool` wheel contained 50,000 small modules and one console entry point; its package initializer imported the last payload module. The large but controlled wheel makes the replacement interval measurable without relying on an unspecified third-party tool or executing package build code. All uv state was isolated with `UV_TOOL_DIR`, `UV_TOOL_BIN_DIR`, `UV_CACHE_DIR`, and `XDG_CONFIG_HOME`, and Python downloads were disabled.

After initially installing the local wheel, the observer ran at the report's 10 Hz sampling rate:

```bash
while true; do
  if PYTHONDONTWRITEBYTECODE=1 "$UV_TOOL_DIR/issue22006-tool/bin/python" \
    -c 'import issue22006_tool' 2>/dev/null; then
    echo OK
  else
    echo MISSING
  fi
  sleep 0.1
done
```

Concurrently, the writer ran:

```bash
uv --no-config tool install \
  --python "$(command -v python3)" \
  --force --reinstall \
  "$RUNNER_TEMP/uv-issue-22006-repro/issue22006_tool-1.0.0-py3-none-any.whl"
```

The 10 Hz observer recorded one failed plain-package import during the 1.72-second reinstall; uv exited 0 and reported that the package and executable were installed. In a second run without the 100 ms sleep, an observer importing the last payload module recorded 34 failures from timestamp 1790478988.682276681 through 1790478989.551131415 (about 0.87 seconds), while uv again exited 0 after 2.33 seconds. The exact duration and number of failures depend on environment size, filesystem, and sampling interval; the report does not name its tool, so its approximately 43-second command duration was not independently compared.

`PYTHONDONTWRITEBYTECODE=1` makes the observer read-only. Without it, the Linux observer created `.pyc` files while uv removed the environment, and that separate variation caused uv to exit 2 with `Directory not empty`, rather than reproducing the reported successful writer.

Existing integration coverage does not test concurrent reader availability. `crates/uv/tests/tool/tool_install.rs`, test `tool_install_force`, verifies that a forced reinstall succeeds and removes a marker from the old environment, and separately verifies `--reinstall`; it does not run a reader while the environment is replaced or assert atomic publication.

## Draft response

Thanks for the concrete report. We reproduced a reader-visible failed import during a successful `uv tool install --force --reinstall` using uv 0.12.13 on Linux. The current tool-install path removes and recreates the installed environment, while the tools-directory lock only serializes uv operations; it does not protect a process that is already reading or running from that environment. We also have prior art for staged, atomically published cached environments in astral-sh/uv#5503 and astral-sh/uv#5509, but that mechanism was not extended to installed tools.

We'll keep this as a distinct bug for making tool replacement safe for concurrent readers. The next step is to add a focused regression test around reader-visible availability and evaluate staged publication without invalidating the absolute interpreter paths embedded in tool entrypoints.

## Classification

This is a bug rather than an enhancement or question. A targeted reproduction observed an incomplete live environment during a successful replacement, and the current source is consistent with the observed behavior:

- Forced tool installation sets the existing environment aside rather than updating it as an existing usable environment.
- `InstalledTools::create_environment` removes the current tool environment before recreating it at the same path.
- The install and upgrade paths contain TODOs to build the environment in the cache and copy it into the tool directory; the install comment specifically identifies absolute interpreter paths in entrypoints as the complication.
- The tools-directory lock is exclusive and serializes uv processes, which explains why concurrent writers need not corrupt each other. It does not provide a read lock to a tool or Python process already using the environment, so it cannot prevent the reported reader-visible gap.

The issue already has the repository's `bug` label. It is not a duplicate: astral-sh/uv#13883 is about coordination among uv subcommands, while astral-sh/uv#5503 was limited to cached environments and was closed by a fix that did not cover installed tools.

## Related

- astral-sh/uv#5503 — Closed issue, "Make `CachedEnvironment` robust to concurrent modifications." This is the closest historical analogue: it identifies modification of an environment while another process uses it, and a maintainer explicitly says the staged-and-linked design might be reusable for tool installs. It covered cached environments, not installed tool environments.
- astral-sh/uv#5509 — Merged pull request, "Add relocatable installs to support concurrency-safe cached environments." It implemented construction in the cache archive followed by publication at a content-addressed location via symlink. This demonstrates a repository-supported atomic-publication pattern, but its changes were confined to cached environments.
- astral-sh/uv#13883 — Open issue, "Stronger locking for parallel operations." It covers filesystem races and reader/writer interactions such as `uv pip list` observing a simultaneous `uv sync`. Its scope and proposed locks assume cooperating uv commands; they do not make an independently running installed tool safe while its environment is replaced.
- astral-sh/uv#12751 — Closed issue, "uv sync failure when running concurrently." This is the reporter's comparison point and concerns multiple `uv sync` writers racing in a project environment. astral-sh/uv#13869 fixed that case by adding locks. The new report differs because writer serialization cannot protect external readers.
- astral-sh/uv#13869 — Merged pull request, "Lock during `uv sync`, `uv add` and `uv remove` to avoid race conditions." This is the fix for astral-sh/uv#12751 and establishes that locking addresses competing uv writers, not continuous availability to processes outside uv.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included `uv tool install --force --reinstall`, `uv tool install` with `atomic`, `reinstall`, `missing`, `partial`, `concurrent`, and `lock`, plus `tool environment`, `site-packages`, and `missing files`. Conceptual queries covered tool upgrades while running, concurrent readers, in-place environment mutation, atomic or relocatable virtual environments, staged construction, symlink publication, and stronger reader/writer locking. Fix-oriented searches included the reporter-cited astral-sh/uv#12751 chain and searches for merged atomic-environment and concurrency fixes.

No closer tool-specific issue or pull request was found. astral-sh/uv#14520 and astral-sh/uv#11134 were inspected as plausible candidates but ruled out: both concern Windows refusing to remove or overwrite executable files that are in use, whereas astral-sh/uv#22006 reports a successful macOS reinstall whose intermediate missing state is visible to readers. astral-sh/uv#5503 and astral-sh/uv#5509 remain the strongest design precedent, and astral-sh/uv#13883 is the closest active but broader concurrency tracker.
