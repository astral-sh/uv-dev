# Builtin creation follows linked environment directories

Issue: astral-sh/uv#23055

Classification: bug

## Summary

The reporter describes uv 0.13.6 on Fedora Linux following pre-existing directory symlinks during `uv venv --allow-existing`. With `v/bin` linked to a sibling directory, interpreter links and activation scripts are created outside `v`. The report also identifies library and site-packages directory components as affected paths.

The supplied example creates `v` and `outside_pkgs`, links `v/bin` to the absolute path of `outside_pkgs`, then runs `uv venv --allow-existing v`. The observed result is `outside_pkgs/activate`. The expected correction is to prevent linked child directories from redirecting environment creation outside its destination.

No exact duplicate found. astral-sh/uv#23056 is the closest companion report, covering file symlinks. Other related work concerns console-script installation, intentional environment-root symlinks, and wheel-write compatibility.

The reporter associates the behavior with CVE-2026-113716 in pypa/virtualenv and says Astral's security contact directed them to report it as a regular bug under uv's threat model. That correspondence is reporter-provided context, not independently verified evidence of uv vulnerability status.

## Draft response

The creation code matches your report: `--allow-existing` retains the existing directory contents, then recursive directory creation and subsequent writes follow a linked `bin` or library directory. This is related to astral-sh/uv#23056, but replacing individual output files would not prevent traversal through a linked parent directory.

The next step is focused regression coverage for linked executable and library directories, followed by deciding whether creation should reject or replace those child links before writing. That needs to retain support for intentional symlinked environment roots, established in astral-sh/uv#14933.

## Classification

The uv 0.13.6 source confirms that --allow-existing retains linked child directories, create_dir_all follows them, and subsequent environment-file writes reach their targets outside the requested environment. This is a correctness problem. The companion file-symlink report requires a different fix, and the historical environment-root and wheel-installation changes did not fix this creation path; no duplicate or regression of a prior fix was established.

astral-sh/uv#23056 was opened shortly afterward by the same reporter and describes symlinks at individual output-file paths. A file-level atomic replacement can address that scenario while still following a symlink in a parent directory. The two reports therefore need distinct coverage and are not treated as duplicates.

The documented meaning of `--allow-existing` explains why existing contents are retained; it does not resolve whether child-directory links should be rejected or replaced. The exact behavior of a fix remains a compatibility decision.

## Related

- astral-sh/uv#23056 — Generated files can follow existing destination links (issue, open). Companion report for uv 0.13.6 and the same `uv venv --allow-existing` command: generated files overwrite targets of pre-existing file symlinks. This report instead follows directory symlinks such as `v/bin`; replacing individual output files atomically would not prevent that parent-directory traversal.
- astral-sh/uv#21255 — uv sync and uv pip install fails for packages with console scripts when the venv contains a `lib` to `usr/lib` symlink (issue, open). Also demonstrates writes redirected by a linked directory inside a virtual environment. It concerns console-script paths during package installation and a subsequent metadata error, whereas this report concerns environment creation itself and writes outside the environment.
- astral-sh/uv#21256 — normalize paths to avoid traversing symlinks when writing console scripts (pull request, open). Proposes a fix for astral-sh/uv#21255 in wheel installation and uninstallation. Review discusses relative paths through linked library directories. It does not change the recursive directory creation or generated-file writes in uv-virtualenv.
- astral-sh/uv#14933 — Fix symlink preservation in virtual environment creation (pull request, merged). Fixed astral-sh/uv#14670 by preserving a symlink at the environment root when clearing or recreating it. Maintainer comments explicitly support following that root symlink. It did not validate linked child directories, so this report is not a regression of that fix.
- astral-sh/uv#21997 — Fix wheel file replacement and stale data handling (pull request, open). Related filesystem-write work in the wheel installer. Its current description explicitly retains destination directory symlinks, and maintainer discussion explains compatibility with distribution-managed layouts. It follows the reverted guard in astral-sh/uv#21569 and concerns a different subsystem from builtin environment creation.

## Supporting evidence

### Current source

The checkout identifies both the uv executable crate and uv-version as 0.13.6.

- `crates/uv-virtualenv/src/virtualenv.rs:110`: the initial metadata check validates the environment location. In the `OnExisting::Allow` branch, existing contents are retained without validating child-directory components.
- `crates/uv-virtualenv/src/virtualenv.rs:252`: the scripts directory is created with `fs_err::create_dir_all(&scripts)`. Unix interpreter links are then written beneath that directory.
- `crates/uv-virtualenv/src/virtualenv.rs:539`: activation templates are written with `fs_err::write(scripts.join(name), activator)`. A symlink in the scripts-directory path redirects these writes.
- `crates/uv-virtualenv/src/virtualenv.rs:600`: the purelib/site-packages path is joined to the environment location and passed to `create_dir_all` at line 602. This likewise does not reject linked directory components. Distutils compatibility files are written there for Python versions below 3.10.
- `crates/uv-cli/src/lib.rs:2971`: `--allow-existing` is documented as writing into the existing location without clearing it.
- `crates/uv-fs/src/lib.rs:1086`: `clear_virtualenv` canonicalizes the environment root before clearing it, consistent with the supported root-symlink behavior discussed in astral-sh/uv#14933.

These findings establish the reported directory traversal in the creation path by source inspection. The supplied reproduction was not executed, and Windows junction behavior was not separately verified.

### Existing coverage and follow-up

`crates/uv/tests/python/venv.rs` contains `non_empty_dir_exists_allow_existing` and `create_venv_then_allow_existing`, which exercise reuse and recreation. It also contains `create_venv_symlink_clear_preservation` and `create_venv_nested_symlink_preservation`, which exercise symlinks at the environment root. The inspected tests do not cover a pre-existing linked `bin` or library directory pointing outside the environment.

A follow-up should add focused integration coverage in that module for executable-directory and library-directory traversal. Each independent scenario should use an explicit command invocation and snapshot, with filesystem assertions that external contents remain unchanged. Platform gates and interpreter-feature gates should follow the surrounding test infrastructure. The behavior chosen for child links should retain supported environment-root symlinks.

### Historical changes and compatibility

astral-sh/uv#14670 described replacement of a symlink at the environment root on repeated creation. Its merged fix, astral-sh/uv#14933, changed clearing/recreation of that root. It did not check child directories and does not establish a regression here.

astral-sh/uv#21569 introduced validation of wheel-installation destinations in September 2026. It was reverted by merged astral-sh/uv#21699 after installation regressions, including astral-sh/uv#21692; a maintainer confirmed that regression was fixed in uv 0.12.15. Closed, unmerged astral-sh/uv#21698 led to open astral-sh/uv#21997. The latter's current description explicitly retains destination directory symlinks, and discussion identifies distribution-managed layouts that legitimately cross installation prefixes. These changes concern wheel installation, not the builtin creator; their history does not establish an earlier fix for this report.

The reporter's upstream reference was inspected directly:
https://github.com/pypa/virtualenv/commit/554d9f9d4f7758f5cf53f993031e29787f1f74c5.
That commit replaces ordinary directory creation with `ensure_within` for executable/library paths and adds tests for existing symlinks and Windows junctions. It supports the technical analogy, but uv's compatibility policy must be considered independently.

## Search scope and exclusions

Searched astral-sh/uv open and closed issues and attempted open, closed, and merged PR searches with authenticated gh. Literal queries included CVE-2026-113716, the upstream commit hash, the title, --allow-existing, pre-planted links, and create_dir_all. Conceptual queries covered venv/virtualenv creation, directory and symbolic links, redirected/outside writes, activation files, junctions, and area:environments/security labels, without Fedora or version restrictions. PR keyword searches returned no candidates even for directly retrievable relevant PRs; supplemented them with paginated open-PR listings, a recent closed-PR listing, and direct inspection of referenced PRs, comments, reviews, and patches. Followed astral-sh/uv#14670 to astral-sh/uv#14933 and astral-sh/uv#21692 through astral-sh/uv#21569, astral-sh/uv#21699, astral-sh/uv#21698, and astral-sh/uv#21997. Ruled out astral-sh/uv#14249 and astral-sh/uv#17963: their fixed --allow-existing regressions concern Windows executable-link creation failing with os error 80. Also ruled out astral-sh/uv#15800 (cross-platform directory aliases) and astral-sh/uv#20431 (build-backend source containment). No inspected item tracks the same child-directory traversal during builtin creation.

Additional inspected context included astral-sh/uv#1863, whose maintainer comment confirms that creating an environment in a non-empty directory is supported via `--allow-existing`, and astral-sh/uv#1472, which discusses clearing existing environments by default. Neither tracks child-directory traversal.

## Verification status

The classification is based on the issue, repository source, existing tests, and the related GitHub discussions and patches. No build or test suite was run. No checkout files or GitHub objects were changed; the only authored artifact is this handoff README.
