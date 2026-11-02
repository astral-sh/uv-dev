# Generated files can follow existing destination links

Issue: astral-sh/uv#23056

Classification: bug

## Summary

The reporter demonstrates that uv 0.13.6 on Fedora Linux overwrites an external file when
`uv venv --allow-existing v` encounters an existing `v/bin/activate` symlink pointing to that
file. The report also identifies activation scripts, `.gitignore`, and `_virtualenv.py/.pth`
as generated destinations with the same write pattern. Expected behavior is to generate the
environment's files without changing the contents of an existing symlink's target.

No confirmed duplicate. astral-sh/uv#23055 is the companion directory-symlink report; astral-sh/uv#5152 and astral-sh/uv#5165 provide the closest file-write precedent. astral-sh/uv#21857 and astral-sh/uv#21891 cover analogous truncation in Git checkout markers.

## Draft response

Your example matches the current source: `--allow-existing` retains existing entries, and the generated-file writes follow destination symlinks. This is a bug. The same write pattern also affects `pyvenv.cfg`; `_virtualenv.py` and `_virtualenv.pth` are generated only for Python below 3.10 in this version.

This is related to astral-sh/uv#23055, which concerns linked parent directories. The next step here is to make generated-file replacement leave symlink targets unchanged and add regression coverage for the external file's contents. astral-sh/uv#5165 provides a precedent for atomic replacement in the wheel installer.

## Classification

The checked-out uv 0.13.6 source leaves existing entries in place for --allow-existing and directly writes activation scripts, .gitignore, pyvenv.cfg, and, for Python below 3.10, _virtualenv.py/.pth. Those writes follow destination-file symlinks and can modify files outside the environment, establishing a correctness bug. The companion directory-link report and historical installer/Git fixes concern distinct paths; there is no evidence that this is a regression of those fixes or a confirmed duplicate.

The distinction from astral-sh/uv#23055 matters: replacing a final file entry can prevent
overwriting that file's symlink target while still following a linked parent directory.
The two reports require separate coverage. The historical fixes predate the reported release,
but changed different subsystems; their dates do not establish a regression here.

The reporter cites CVE-2026-113721 and pypa/virtualenv commit
`d25f28699eefb1fc34a251e5adf2b5de844c36cd`, and says Astral's security contact considered the
scenario outside uv's threat model. These are reporter-provided context, not independently
verified CVE applicability or a new security assessment. The source-backed correctness
finding does not depend on that characterization.

## Related

- astral-sh/uv#23055 — Builtin creation follows linked environment directories (open issue). Companion report for uv 0.13.6 and `uv venv --allow-existing`: a pre-existing directory symlink redirects generated subtrees outside the environment. This report instead concerns individual destination-file symlinks; replacing those files does not address traversal through linked parent directories.
- astral-sh/uv#5152 — Uv script install into .venv overwrites python (closed issue). Confirmed historical example of a generated script following an existing file symlink and overwriting its external target. It involved installing a console entry point named python during `uv sync`, rather than generating virtual-environment files, and was fixed by astral-sh/uv#5165.
- astral-sh/uv#5165 — Make entrypoint writes atomic to avoid overwriting symlinks (merged pull request). Merged July 17, 2024; replaced direct entry-point writes with atomic writes to fix astral-sh/uv#5152. Its changes were confined to the wheel installer, providing a relevant implementation precedent without fixing the uv-virtualenv writes reported here.
- astral-sh/uv#21857 — uv Git checkout marker follows a repository-controlled symlink (closed issue). Another confirmed external-file truncation through an existing destination symlink, but triggered by writing a Git checkout's .ok marker during dependency installation. Its canonical fix is astral-sh/uv#21891; it does not cover virtual-environment generation.
- astral-sh/uv#21891 — Store Git checkout markers outside working trees (merged pull request). Merged September 23, 2026; fixes astral-sh/uv#21857 by moving checkout markers outside repository-controlled working trees. Maintainer comments on the closed alternative astral-sh/uv#21892 explicitly select this fix. It changes the Git/cache subsystem, not destination-file writes in uv-virtualenv.

## Supporting evidence

The checkout is at commit `39a02a0`, with version `0.13.6` in `crates/uv/Cargo.toml:3`.
The changelog dates that release to October 26, 2026. Both current reports were opened on
November 2, 2026 and had no comments when inspected.

- `crates/uv-virtualenv/src/virtualenv.rs:145`: `OnExisting::Allow` permits the existing
  directory without clearing its entries.
- `crates/uv-virtualenv/src/virtualenv.rs:215`: `.gitignore` uses `fs_err::write`.
- `crates/uv-virtualenv/src/virtualenv.rs:539`: activation templates use `fs_err::write`.
- `crates/uv-virtualenv/src/virtualenv.rs:596`: `pyvenv.cfg` uses `File::create`.
- `crates/uv-virtualenv/src/virtualenv.rs:621`: the distutils patch files use
  `fs_err::write`; the condition at line 50 limits their creation to Python below 3.10.
- These ordinary create/truncate writes follow a destination symlink. There is no unlink or
  atomic replacement at these call sites.
- `crates/uv-install-wheel/src/wheel.rs:912` uses `uv_fs::write_atomic_sync` for recorded
  generated files, consistent with the installer-specific precedent in astral-sh/uv#5165.
  The helper at `crates/uv-fs/src/lib.rs:558` writes a temporary file and renames it.
- The changed-file lists for astral-sh/uv#5165 and astral-sh/uv#21891 do not include
  `uv-virtualenv`. Maintainers explicitly selected astral-sh/uv#21891 over the closed,
  unmerged alternative astral-sh/uv#21892.

This is source confirmation, not a newly executed reproduction. No build or test was run,
and no checkout file or GitHub object was changed.

## Follow-up and existing coverage

The implementation follow-up is to replace generated destination files without modifying
their symlink targets, using the installer precedent where appropriate. Keep directory-link
handling coordinated with astral-sh/uv#23055; atomic final-file replacement alone does not
solve that separate case.

Existing tests in `crates/uv/tests/python/venv.rs` cover repeated creation with
`--allow-existing` (`non_empty_dir_exists_allow_existing` at line 1691 and
`create_venv_then_allow_existing` at line 1747), root symlink preservation, and conditional
distutils patch generation. The inspected cases do not pre-plant a generated-file symlink
and verify that its external target remains unchanged.

Add focused integration coverage in that existing module, using explicit setup and
`uv_snapshot!` command output in the surrounding style. Check the external file's complete
contents after creation. The module already requires `test-python`; Unix symlink cases
need the appropriate platform gate, and a Python 3.9 patch-file case should follow the
existing `test-python-eol` gate. No tests were added during this handoff.

## Search scope and exclusions

The report was decomposed before searching into the observable external-file overwrite;
the `uv venv --allow-existing` trigger; each generated-file family; the existing
destination-file symlink condition; and the exact CVE identifier. Searches for the
observable behavior were kept separate from searches for atomic-write or link-handling
mechanisms. Broader searches omitted Fedora, the uv version, and incidental file names.

Used authenticated gh to search astral-sh/uv issues across open and closed states and pull requests without a state restriction, with additional explicit open, closed, and merged PR attempts. Literal searches covered CVE-2026-113721, destination links, --allow-existing with symlink, activate, .gitignore, and _virtualenv.py. Conceptual and fix-oriented searches covered symbolic links, overwrite, truncate, clobber, hardlinks, atomic writes, generated files, paths outside the environment, and area:environments. Inspected candidate bodies, comments, reviews, timelines, and changed-file lists; followed astral-sh/uv#5152 to astral-sh/uv#5165 and astral-sh/uv#21857 through astral-sh/uv#21892 to astral-sh/uv#21891. Ruled out astral-sh/uv#14670 and astral-sh/uv#14933 as duplicates: they preserve the environment-root symlink during recreation, not individual generated-file targets. PR search results were incomplete despite directly retrievable historical PRs, and later REST searches hit an installation rate limit; direct GraphQL PR inspection remained available. No exact duplicate or prior fix for this virtualenv-file path was confirmed.

astral-sh/uv#14670 and its merged fix astral-sh/uv#14933 were particularly plausible from
their titles, but their desired behavior is to retain and follow an environment-root
symlink while recreating its target. Maintainer comments confirm that intent. They do not
track writes through symlinks at generated file destinations. The broader
astral-sh/uv#1472 discussion concerns clearing an existing environment, not this final-file
write behavior.

Search limitations mean that the absence of an exact duplicate is not an exhaustive claim.
The strongest relationships above were verified through direct issue and pull request views.
