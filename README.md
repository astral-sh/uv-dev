# Implicit inheritance of `uv.tool` section in pyproject.toml across repositories causes confusion between multiple projects in nested checkouts

Issue: astral-sh/uv#22185

Classification: bug (reported configuration-isolation concern; behavior reproduced)

## Summary

Two independent Python repositories normally live side by side, but GitLab CI requires the second checkout to live inside the first. Both have their own `pyproject.toml`. The parent declares `[[tool.uv.dependency-metadata]]`; the child has no `[tool.uv]` table. The reporter expects the child's independently generated lockfile to remain valid when the checkout is nested, but reports that `uv lock --check` rejects it. Adding an empty `[tool.uv]` to the child avoids the failure. The title says `uv.tool`; the actual configuration table is `tool.uv`.

The behavior was reproduced using installed uv 0.12.13 on Linux with CPython 3.12.3. A dependency-free child passes lock validation as a sibling and fails after being moved inside a separate parent Git repository. Verbose output identifies the parent configuration and a static-metadata mismatch. An empty child `[tool.uv]` table and `--no-config` independently restore success without changing the child's lockfile.

The report lists uv 0.11 and 0.12, with continued behavior through 0.12.23 explicitly described as an assumption. Platform and Python version are reported as independent. Exact project files and command output were not supplied. The reproduction demonstrates a minimal sufficient configuration; it does not verify every reported version or platform.

## Classification

Retain the bug classification as a report of surprising configuration coupling between independent projects. Reproduction establishes the observed behavior, not that documented ancestor discovery is inherently incorrect. The documentation explicitly says that a `pyproject.toml` without `[tool.uv]` is skipped and discovery continues up the directory hierarchy. Whether independent Git checkouts should introduce an additional boundary is a maintainer design decision.

This is not an established regression: the report gives no last known-good release, and related history concerns different boundaries or configuration inputs. The child remains its own project/workspace root in the runtime logs; the inherited settings come from the separate configuration-discovery step. This selects the nearest applicable project configuration rather than merging every ancestor's table.

No exact duplicate was identified in the retained issue search. Configuration isolation and pruning unused lock inputs are related but separate concerns. The reproduction proves that an unused parent metadata entry is sufficient on uv 0.12.13; the reporter's real metadata may also describe dependencies used by the child.

## Related

- astral-sh/uv#5931 — Search beyond workspace root when discovering configuration (pull request, merged). Introduced configuration discovery above workspace roots. Maintainers explicitly discussed ignoring parent pyproject.toml files while continuing to read parent uv.toml files, but deferred that change. This explains the relevant history; it did not fix independent-checkout isolation.
- astral-sh/uv#5929 — Parent uv.toml configuration not used when pyproject.toml present (issue, closed). A child pyproject.toml without uv settings incorrectly blocked intentional parent uv.toml discovery; astral-sh/uv#5931 fixed it in August 2024. The new report concerns unwanted parent pyproject.toml settings crossing independent checkout boundaries, so it is neither a duplicate nor a regression of that opposite failure.
- astral-sh/uv#21894 — Omit unused dependency-metadata entries from lockfiles (pull request, closed). Proposed preventing unused static-metadata declarations from invalidating lockfiles, directly relevant if the inherited entries are unused by the child. Closed without merging before this report; it does not restrict configuration discovery, and the report does not establish that every inherited entry is unused.
- astral-sh/uv#19196 — Configuration merging produces unusable uv.lock files (issue, open). Also reports configuration outside a project making lock validation fail in CI. Its trigger is user-level exclude-newer-package settings, rather than dependency-metadata discovered in a separate parent checkout. The related astral-sh/uv#20652 was explicitly centralized there; independent-checkout discovery remains a distinct problem.

## Reproduction

**Outcome: reproducible.**

### Environment and isolation

- Executable from `PATH`: `/opt/hostedtoolcache/uv/0.12.13/x86_64/uv`; version: `uv 0.12.13 (x86_64-unknown-linux-gnu)`.
- OS: Linux x86_64, kernel `6.17.0-1022-azure`; interpreter: `/usr/bin/python3`, CPython 3.12.3.
- All fixture files, Git repositories, cache, configuration directories, and logs were created under `/tmp/uv-22185-_fb3zz67`.
- Commands ran with a clean, explicitly constructed environment. `UV_CACHE_DIR`, XDG configuration/cache/data paths, `UV_PYTHON_INSTALL_DIR`, and `TMPDIR` pointed inside that temporary directory. `UV_NO_SYSTEM_CONFIG=1` disabled system configuration, and `UV_PYTHON_DOWNLOADS=never` prevented interpreter downloads. Git used an empty template directory and disabled global/system configuration.
- All uv commands used `--offline` and `--python /usr/bin/python3`. No dependencies, network requests, dependency groups, workspace declarations, sources, or preview settings were needed. Each project had its own real `.git` directory.
- An empty `uv.toml` at the temporary fixture root bounded ancestor discovery when the projects were siblings. The nested child found the parent's `pyproject.toml` before reaching that empty file.

### Minimal configuration and commands

Start with sibling directories `parent/` and `child/`, each initialized with `git init`. The parent has:

```toml
[project]
name = "parent-project"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []

[[tool.uv.dependency-metadata]]
name = "parent-only-package"
version = "1.0.0"
requires-dist = []
```

The child has:

```toml
[project]
name = "child-project"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []
```

From the fixture root, run these commands with the isolated environment described above:

```sh
(cd parent && uv --offline lock --python /usr/bin/python3)
(cd parent && uv --offline lock --check --python /usr/bin/python3)
(cd child && uv --offline lock --python /usr/bin/python3)
(cd child && uv --offline lock --check --python /usr/bin/python3)
mv child parent/child
(cd parent/child && uv --offline lock --check -v --python /usr/bin/python3)
(cd parent/child && uv --offline lock --check --no-config --python /usr/bin/python3)
printf '\n[tool.uv]\n' >> parent/child/pyproject.toml
(cd parent/child && uv --offline lock --check -v --python /usr/bin/python3)
```

### Observed results

| Scenario | Exit code | Result |
| --- | --- | --- |
| Parent lock creation and check | 0 | Resolves its one local project successfully |
| Sibling child lock creation and check | 0 | Resolves its one local project successfully |
| Identical child and lockfile moved into parent | 1 | Lockfile needs updating |
| Nested child with `--no-config`, before adding its empty table | 0 | Original lockfile accepted |
| Nested child with empty `[tool.uv]`, normal discovery | 0 | Original lockfile accepted |

Relevant nested failure output, with the temporary root normalized to `<repro>`:

```text
DEBUG Project is contained in non-workspace project: `<repro>/parent`
DEBUG Found workspace root: `<repro>/parent/child`
DEBUG Skipping `pyproject.toml` in `<repro>/parent/child` (no `[tool]` section)
DEBUG Found workspace configuration at `<repro>/parent/pyproject.toml`
DEBUG Resolving despite existing lockfile due to mismatched static metadata:
  Requested: {StaticMetadata { name: PackageName("parent-only-package"), version: Some("1.0.0"), requires_dist: [], requires_python: None, provides_extra: [] }}
  Existing: {}
error: The lockfile at `uv.lock` needs to be updated, but `--check` was provided.

hint: To update the lockfile, run `uv lock`.
```

With the empty child table, verbose output instead selects `<repro>/parent/child/pyproject.toml` and reports `Existing uv.lock satisfies workspace requirements` (the actual log puts backticks around `uv.lock`). The child's lockfile remained byte-for-byte unchanged after the failing check and both successful workaround checks. The child has no dependencies, so the inherited metadata is demonstrably unused in this fixture. The child's Git repository boundary did not stop configuration discovery.

The fixture and complete command output remain in the temporary directory above. `results.json` records all seven invocations, exit codes, stdout, and stderr; `child-original.lock` preserves the original child lockfile. No runtime check was made with uv 0.11, uv 0.12.23, another operating system, or preview options. No comparison with a last known-good version applies because none was reported.

### Existing test coverage

The following test setup and assertions were read; these repository tests were not executed or modified:

- `crates/uv/tests/sync/show_settings.rs`, `resolve_skip_empty`: creates a parent `uv.toml` setting pip resolution to `lowest-direct` and a child `pyproject.toml` without `[tool.uv]`; snapshots show inherited resolution. Adding an empty child table restores baseline settings. It covers the general discovery rule and empty-table behavior, not parent `pyproject.toml` metadata or independent nested Git repositories. The module requires `test-python` and `test-pypi`; this test is ignored on Windows.
- `crates/uv/tests/lock/lock.rs`, `lock_dependency_metadata`: records an `anyio` metadata override in the lock manifest, accepts unchanged metadata with `--locked` (including offline validation), and updates the dependency graph when metadata changes or is removed. It does not test ancestor configuration discovery. The test additionally requires `test-universal`.
- `crates/uv/tests/lock/lock.rs`, `lock_resolution_inputs_prune_unused_inputs`: with the `resolution-inputs` preview enabled when creating the lock, snapshots omit unused metadata and other irrelevant settings, and later locked/offline validation succeeds after those unused settings change. Removing a relevant exclusion fails. This is opt-in pruning coverage in the inspected checkout, not evidence that nested configuration discovery is bounded or that the default reproduction is fixed. The test additionally requires `test-universal`.

Searches covered `crates/uv/tests/` and `crates/uv-client/tests/it/`. No inspected test combines independent nested repositories, parent static metadata, and the child's independently created lockfile. No new test was added because this task is reproduction only.

## Supporting implementation and documentation

Source and tests were inspected at checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`; the runtime experiment used the separately installed uv 0.12.13 executable.

- `crates/uv/src/lib.rs:339` starts settings discovery at the discovered workspace's install path. Project/workspace discovery and settings discovery are separate operations.
- `crates/uv-settings/src/lib.rs:102` traverses `path.ancestors()` until configuration is found, without a Git boundary check. `from_directory` skips a `pyproject.toml` without `[tool.uv]`; an empty table supplies an options object and stops ancestor lookup.
- `docs/concepts/configuration-files.md` documents skipping files without that table, discovering ancestor configuration, merging project/user/system settings, and disabling discovery with `--no-config`.
- `crates/uv-lock/src/lock/mod.rs:4486` compares configured static metadata, after the applicable lock filter, with recorded manifest metadata. `crates/uv-lock-operations/src/validated_lock.rs:429` logs the observed `mismatched static metadata` diagnostic.
- The historical discussion in astral-sh/uv#5931 considered limiting above-workspace lookup to `uv.toml`, but deferred that change. This is design history, not a completed independent-checkout boundary fix.

These sources explain the directly observed behavior; they are not being used as a substitute for runtime reproduction.

## Search coverage and exclusions

The following related-issue search record is retained from the issue-context investigation. It was not rerun during the local reproduction:

Searched astral-sh/uv open and closed issues with authenticated gh: literal tool.uv/parent, dependency-metadata/lock/outdated, lock --check/parent, empty tool.uv, and mismatched static metadata; conceptual nested repositories, configuration inheritance/discovery, ancestor traversal, Git/filesystem boundaries, independent projects, and configuration-dependent CI lock failures. Checked version-specific and historical fixes and followed candidate comments and closing PRs. REST search was rate-limited and GraphQL PR searches returned no PR nodes, so enumerated all 12,901 PRs (602 open, 1,428 closed, 10,871 merged), searched their titles for discovery, boundaries, inheritance, static metadata and lock invalidation, then inspected relevant bodies/comments. Ruled out astral-sh/uv#11070 (explicit configuration merging), astral-sh/uv#7351 (filesystem mount boundaries), astral-sh/uv#16502 (explicit workspace membership), astral-sh/uv#3936 and astral-sh/uv#8665 (cached dependency workspace discovery), and astral-sh/uv#13614 and astral-sh/uv#13635 (marker-normalization regression). No exact duplicate or previously merged fix for independent-checkout configuration isolation was found.

The search vocabulary also used the repository's `area:configuration`, `area:workspaces`, `area:lock`, and `area:error-messages` distinctions. Observable lock-check failures were searched separately from the proposed discovery mechanism. Broad searches omitted GitLab, uv versions, and platform details to find the underlying behavior. PR title enumeration was a fallback for unavailable reliable PR text search; relevant candidates were then read in full, including their comments. It is not a claim to have searched every PR comment.

Other inspected candidates do not establish a duplicate or regression:

- astral-sh/uv#11070 requests explicitly merging ancestor configuration even when a child already has its own `[tool.uv]`; astral-sh/uv#5596 requests an `extend` option. Their goal differs from preventing accidental ancestor discovery. The referenced astral-sh/uv#10960 is broader monorepo documentation.
- astral-sh/uv#7351 concerns crossing filesystem mount boundaries and an NFS-created directory named `uv.toml`. Stopping at a mount would not isolate nested repositories on the same filesystem.
- astral-sh/uv#16502 declares the submodule as a workspace member and requests an independent lockfile. Maintainer advice there is to use a path dependency. The current report describes independent projects, not declared workspace membership.
- astral-sh/uv#3936 and its merged fix astral-sh/uv#8665 bound workspace discovery for cached Git dependencies. They do not bound the top-level filesystem settings lookup used here.
- astral-sh/uv#13614 and its merged fix astral-sh/uv#13635 concern equivalent dependency markers comparing differently during lock validation. Their same broad lock-check error does not establish a regression in nested-checkout discovery.
- astral-sh/uv#18053 and astral-sh/uv#18087 concern named-index visibility and a diagnostic hint, not inherited static metadata. astral-sh/uv#21284 improves requirement-mismatch diagnostics after metadata collection fails, rather than identifying or bounding inherited configuration.
- astral-sh/uv#20652 was closed explicitly as a duplicate of astral-sh/uv#19196, so the latter is the canonical adjacent discussion for user-level cooldown configuration in lockfiles.

## Maintainer next step and verification limits

The reported failure and empty-table workaround are confirmed. Keep the independent-checkout discovery question distinct from unused-input pruning: a discovery change would need to account for intentional sharing of parent `uv.toml` configuration, while pruning cannot isolate parent settings that actually affect the child.

An empty child `[tool.uv]` table stops ancestor project lookup but still allows user/system settings. `uv lock --check --no-config` also passed in this fixture; it is appropriate only when disabling all discovered uv settings is acceptable. Verbose output identifies both the selected configuration path and the differing metadata.

A maintainer can now assess the intended configuration boundary and diagnostics using the concrete fixture. The reported platform independence and additional uv versions remain unverified. Existing preview pruning tests should be considered before claiming that all current configurations invalidate locks for unused metadata.

No checkout files or existing user state were modified, no builds were performed, and no GitHub changes were made. Only temporary reproduction artifacts and this issue-context README were written. The pre-existing modifications to `agents/codex/config.toml` and untracked `.issue-triage-event.json` were left untouched.
