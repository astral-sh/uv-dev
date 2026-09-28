# uv tool install --force --reinstall mutates the tool environment in place: concurrent readers observe missing files for tens of seconds while the install exits 0

Issue: astral-sh/uv#22006

Classification: enhancement

## Summary

The reported reader-visible interruption is reproducible. With an isolated tool directory and a controlled local wheel, an external process repeatedly importing from the installed tool environment observed a failed import during `uv tool install --force --reinstall`, while uv 0.12.13 completed successfully. A faster observer targeting a module installed late in the wheel saw 34 failures over about 0.87 seconds during another successful reinstall.

A maintainer has classified uninterrupted tool availability during an update as a feature request rather than an existing correctness guarantee. They also identified an important scope limitation in the proposed atomic publication approach: it can ensure that new path lookups see a complete old or new environment, but a process that already holds references to directories in the old environment may still encounter missing files after that old environment is deleted. The implementation in astral-sh/uv-dev#2118 therefore addresses the supplied observer reproduction on Linux and macOS but does not by itself establish that every already-running tool remains functional throughout replacement and cleanup.

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

## Proposed fix and scope

Outcome: **the supplied path-based reader reproduction is fixed by the proposed implementation** on Linux and macOS; availability for all already-running processes remains unresolved.

The root cause was confirmed in the forced-install branch: setting `--force` discards the existing environment selection, and `InstalledTools::create_environment` removed the installed directory before `sync_environment` populated its replacement. The tools-directory lock serializes uv writers but cannot prevent external readers from resolving the stable tool path during that interval.

In astral-sh/uv-dev#2118, forced replacement of a valid installed tool on Linux and macOS creates and synchronizes a staged virtual environment in a sibling directory. Generated virtual-environment paths are rewritten from the staging location to the stable tool destination, then a new `uv_fs::exchange_paths` primitive uses the platform's atomic rename-exchange operation to publish the completed directory without making the destination disappear. Existing executable links remain in place during preparation and publication; after finalization, only entrypoints absent from the new receipt are removed. New installs and platforms without atomic directory exchange retain their prior path.

This publication strategy protects readers that resolve the stable path after the exchange: they see either the complete old tree or the complete new tree. Per the maintainer's follow-up, it does not necessarily protect a long-running process that retained a directory reference into the old tree and opens additional files after cleanup deletes that tree. Investigation should therefore distinguish fresh readers like the current regression test from already-running processes, and define the intended lifetime or reclamation policy for replaced environments before treating uninterrupted execution as fully solved.

The proposed integration test `tool_install_force_reinstall_preserves_concurrent_import` asserts that a continuously reloaded import never disappears during `--force --reinstall`, and it runs the installed console entrypoint after replacement. The neighboring `tool_install_force` test caught and prevented a relocatable-script approach that would have required `dirname` and `realpath` on `PATH`; the proposed implementation retains the existing absolute console-script form.

Focused validation passed:

- `cargo test --package uv --test tool tool_install::tool_install_force_reinstall_preserves_concurrent_import -- --exact`
- `cargo test --package uv --test tool tool_install::tool_install_force -- --exact`
- `cargo test --package uv --test tool tool_install::tool_install_already_installed -- --exact`
- `cargo test --package uv --test tool tool_install::tool_install_force_respects_global_python_change -- --exact`
- `cargo +stable clippy --package uv --test tool --locked -- -D warnings`
- `cargo +stable fmt --all -- --check`

The pinned 1.98.1 toolchain did not have rustfmt or clippy installed and its read-only installation could not be extended, so the available stable rustfmt and clippy components were used. `cargo-xwin` was unavailable, but the Windows path remains behind the existing non-atomic implementation and the new staging and exchange code is target-gated to Linux and macOS.

## Classification

This is classified as an enhancement. A targeted reproduction confirms the reader-visible interruption, but a maintainer stated that continuous tool availability during an update is not necessarily an existing expectation and explicitly marked the issue as a feature request. That project decision supersedes the initial bug triage.

The source and reproduction still establish the current mechanism and the narrower benefit of atomic publication:

- Forced tool installation sets the existing environment aside rather than updating it as an existing usable environment.
- The replacement branch formerly called `InstalledTools::create_environment`, which removed the current tool environment before recreating it at the same path.
- The proposed Linux and macOS implementation synchronizes in a sibling directory, rewrites generated paths, and uses atomic directory exchange at publication.
- The tools-directory lock is exclusive and serializes uv processes, which explains why concurrent writers need not corrupt each other. It does not provide a read lock to a tool or Python process already using the environment, so it cannot prevent the reported reader-visible gap.
- Atomic exchange protects new resolution of the stable path, but it does not guarantee later file access through references an existing process retained into the old environment once that environment is deleted.

It is not a duplicate: astral-sh/uv#13883 is about coordination among uv subcommands, while astral-sh/uv#5503 was limited to cached environments and was closed by a fix that did not cover installed tools.

## Related

- astral-sh/uv#5503 — Closed issue, "Make `CachedEnvironment` robust to concurrent modifications." This is the closest historical analogue: it identifies modification of an environment while another process uses it, and a maintainer explicitly says the staged-and-linked design might be reusable for tool installs. It covered cached environments, not installed tool environments.
- astral-sh/uv#5509 — Merged pull request, "Add relocatable installs to support concurrency-safe cached environments." It implemented construction in the cache archive followed by publication at a content-addressed location via symlink. This demonstrates a repository-supported atomic-publication pattern, but its changes were confined to cached environments.
- astral-sh/uv#13883 — Open issue, "Stronger locking for parallel operations." It covers filesystem races and reader/writer interactions such as `uv pip list` observing a simultaneous `uv sync`. Its scope and proposed locks assume cooperating uv commands; they do not make an independently running installed tool safe while its environment is replaced.
- astral-sh/uv#12751 — Closed issue, "uv sync failure when running concurrently." This is the reporter's comparison point and concerns multiple `uv sync` writers racing in a project environment. astral-sh/uv#13869 fixed that case by adding locks. The new report differs because writer serialization cannot protect external readers.
- astral-sh/uv#13869 — Merged pull request, "Lock during `uv sync`, `uv add` and `uv remove` to avoid race conditions." This is the fix for astral-sh/uv#12751 and establishes that locking addresses competing uv writers, not continuous availability to processes outside uv.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries included `uv tool install --force --reinstall`, `uv tool install` with `atomic`, `reinstall`, `missing`, `partial`, `concurrent`, and `lock`, plus `tool environment`, `site-packages`, and `missing files`. Conceptual queries covered tool upgrades while running, concurrent readers, in-place environment mutation, atomic or relocatable virtual environments, staged construction, symlink publication, and stronger reader/writer locking. Fix-oriented searches included the reporter-cited astral-sh/uv#12751 chain and searches for merged atomic-environment and concurrency fixes.

No closer tool-specific issue or pull request was found. astral-sh/uv#14520 and astral-sh/uv#11134 were inspected as plausible candidates but ruled out: both concern Windows refusing to remove or overwrite executable files that are in use, whereas astral-sh/uv#22006 reports a successful macOS reinstall whose intermediate missing state is visible to readers. astral-sh/uv#5503 and astral-sh/uv#5509 remain the strongest design precedent, and astral-sh/uv#13883 is the closest active but broader concurrency tracker.

Proposed implementation: astral-sh/uv-dev#2118
