# uv python pin and uv lock write through repo-shipped .python-version / uv.lock symlinks

Issue: astral-sh/uv#23319

Classification: `bug`

## Summary

On uv 0.13.9, the reporter found that `uv python pin` and project lockfile updates from `uv lock` or `uv sync` follow symlinks at `.python-version` or `uv.lock`. A project can thereby direct a write outside its workspace. The closest history is the two atomic-write changes for these files and separate symlink-write reports for Git checkout markers and generated virtualenv files. None tracks or fixes the reported project-state writes.

## Draft response

Thanks for reporting this. Both writers preserve the symlink and write to its target. `uv python pin` doesn't require the target to contain a valid pin; `uv lock` and `uv sync` stop if an existing target can't be parsed as a lockfile. The atomic-write changes in astral-sh/uv#22399 and astral-sh/uv#22402 prevent partial reads, not writes outside the project. You still need to trust projects before running commands such as `uv lock`. The next step is to decide how these writers should handle out-of-project links without breaking intentional symlink setups. For now, use regular project files where you want the updates to stay local.

## Classification

This is a bug: the pin and lock writers can modify out-of-project files selected by repository-shipped links. Deliberate link preservation explains the behavior but does not resolve the project-state write boundary. The atomic-write PRs kept link following while addressing partial reads; their predecessor writes also followed links. The Git checkout marker fix was for a different writer, so this is neither a duplicate nor a regression of that fix. The maintainer's trust warning for commands such as `uv lock` is relevant to security scope, but separate from the file-write behavior. This classification makes no security advisory determination.

## Related

- astral-sh/uv#22399 (merged) — **Write lockfiles atomically for concurrent frozen readers.** Introduced the shared writer and used it to publish project and script lockfiles atomically. Preserving symlinks was explicit; the fix was for partial reads, not where writes are allowed.
- astral-sh/uv#22402 (merged) — **Write Python version files atomically.** Reused that writer for `.python-version` and explicitly kept link preservation. It fixed a concurrent reader seeing an empty pin, not the destination of the write.
- astral-sh/uv#21857 (closed) — **uv Git checkout marker follows a repository-controlled symlink.** Same class of out-of-checkout writes through a tracked symlink, but the file was uv's `.ok` readiness marker in a cached Git dependency checkout, not project state.
- astral-sh/uv#21891 (merged) — **Store Git checkout markers outside working trees.** Fixed astral-sh/uv#21857 by moving the marker outside the dependency working tree. Released in 0.12.19; it did not change pin or lockfile writes.
- astral-sh/uv#23056 (open) — **Generated files can follow existing destination links.** Similar external overwrite, but limited to generated virtualenv files under `uv venv --allow-existing`. Maintainers discussed it as a normal behavior report rather than a vulnerability in that threat model.
- astral-sh/uv#23104 (closed, unmerged) — Proposed atomically replacing destination links for generated virtualenv files without following them. It addresses astral-sh/uv#23056 only and did not change either project-state writer.

## Source evidence

- `crates/uv-python-commands/src/pin.rs:221` constructs the local pin path and writes the requested version without requiring the existing contents to parse. `crates/uv-python-discovery/src/version_files.rs:289` delegates the write to `write_atomic_preserve`.
- `crates/uv-lock-operations/src/lock_target.rs:389` parses an existing lock before the write path can commit. A TOML parse failure aborts the operation; a changed resolution in write mode is committed through the same helper. `uv sync` reaches that path when it updates the project lock, not in frozen or locked mode.
- `crates/uv-fs/src/lib.rs:501` documents link preservation. The helper resolves the link before publication, does not enforce project containment, and can create a missing target where the parent directory exists and permits the write. Existing targets must be regular files. Changes are published by atomic replacement at the resolved target, not by truncation in place.
- The writer paths were checked in the 0.13.9 release source and in the checkout. The atomic-write fixes were released in 0.13.1 on October 14, 2026, before the reported 0.13.9 release on November 17, 2026.

## Search scope

Authenticated GitHub CLI searches covered open and closed issues and open, closed, and merged pull requests in astral-sh/uv. Pin searches used `uv python pin`, `.python-version`, and version-file destination links. Lock searches used `uv.lock`, `uv lock`, `uv sync`, and `` Failed to parse `uv.lock` ``, plus lockfile symlinks and outside destinations. Separate fix searches covered `write_atomic_preserve`, dangling targets, repository-controlled links, overwrite, atomic publication, and 0.13.9. Candidate discussions, PR reviews, linked changes, and release history were checked. The recent pull list was also scanned as a fallback when PR search returned no known matches.

The discussion in astral-sh/uv#19222 and its proposed fixes astral-sh/uv#21981 and astral-sh/uv#23035 concern lookup beside a symlinked PEP 723 script, not writes through a symlink at `uv.lock`. The change in astral-sh/uv#22403 shares the atomic helper but updates managed Python sysconfig files; astral-sh/uv#22487 and astral-sh/uv#22496 concern special files chosen as command output destinations. The proposal in astral-sh/uv#23160 is a second unmerged approach limited to generated virtualenv files, and astral-sh/uv#23055 concerns linked environment directories rather than links at project-state filenames.
