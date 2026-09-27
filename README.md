# uv tool install --force --reinstall mutates the tool environment in place: concurrent readers observe missing files for tens of seconds while the install exits 0

Issue: astral-sh/uv#22006

Classification: bug

## Summary

The report shows that `uv tool install --force --reinstall <tool>` can leave the installed tool environment temporarily incomplete while the command is replacing it. An external process repeatedly importing from that environment observed missing modules during the successful reinstall. Running two reinstalls extends the reader-visible disruption even though both uv invocations report success.

No existing issue specifically tracks atomic publication of installed tool environments. The closest historical work is astral-sh/uv#5503 and astral-sh/uv#5509, which made cached environments safe to recreate while in use by constructing them elsewhere and publishing them through a symlink. astral-sh/uv#13883 tracks stronger locking between uv operations, but external processes already running from a tool environment cannot participate in uv's locks.

## Draft response

Thanks for the concrete reproduction. The current tool-install path does remove and recreate the installed environment when `--force` is used, while the tools-directory lock only serializes uv operations; it does not protect a process that is already reading or running from that environment. We also have prior art for staged, atomically published cached environments in astral-sh/uv#5503 and astral-sh/uv#5509, but that mechanism was not extended to installed tools.

We'll keep this as a distinct bug for making tool replacement safe for concurrent readers. The next step is to add a focused regression test around reader-visible availability and evaluate staged publication without invalidating the absolute interpreter paths embedded in tool entrypoints.

## Classification

This is a bug rather than an enhancement or question. The reported successful replacement exposes an incomplete live environment to readers, and the current source confirms the relevant behavior:

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
