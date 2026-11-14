# Opt-in, read-only discovery of inherited packages on `sys.path` for `uv pip`

Issue: astral-sh/uv#23242

Classification: duplicate

## Summary

The reporter proposes an opt-in `sys-path-discovery` preview for `uv pip list`, `freeze`, `show`, `tree`, `check`, `install`, and `uninstall`. Inherited distributions would satisfy requirements but remain read-only; only the target environment's `purelib` and `platlib` would be mutable. A fresh interpreter query would discover ordered runtime paths, including `--system-site-packages`, `.pth` files, `sitecustomize.py`, and `PYTHONPATH`.

The motivating example layers a lightweight project environment over a read-only ML/container/HPC base with `torch`, CUDA dependencies, and ABI-coupled extensions. The reporter says scheme-only discovery causes redundant downloads and environment-local shadowing that can break those extensions. The proposal allows shadowing incompatible inherited versions only when the environment has import precedence; otherwise it errors. Exact installation would remove only mutable extraneous distributions, and uninstall would skip inherited distributions with a warning.

`uv pip sync`, project commands, tool and build environments, `--target`, and `--prefix` are excluded. The reporter supplies a prototype in Hangzhi/uv#1 and asks for agreement on full runtime discovery versus explicit paths, shadowing versus failure on mismatches, and the limited command scope.

Duplicate of astral-sh/uv#2500, with the install symptom tracked by astral-sh/uv#4466. Prior discussions already cover the proposed limited scope; astral-sh/uv#9849 remains a draft and astral-sh/uv#11670 closed unmerged. Full-path discovery and mismatch behavior remain design questions.

## Draft response

Your proposed opt-in, read-only support for uv pip fits the discussion in astral-sh/uv#2500; astral-sh/uv#4466 tracks the redundant-install behavior, including HPC environments with preinstalled torch. Please add your proposal and prototype to astral-sh/uv#2500 so the design discussion stays together.

The earlier discussion supports exploring a limited first step, but konstin suggested explicit paths and failing on version mismatches. Full sys.path discovery and shadowing inherited versions still need agreement. Astral-sh/uv#9849 remains a draft, astral-sh/uv#11670 closed without merging, and astral-sh/uv#15121 only changed uv run --with overlays.

## Classification

The underlying inherited-package discovery and redundant-install behavior is already tracked by open astral-sh/uv#2500 and astral-sh/uv#4466. Astral-sh/uv#2500 also already discusses the proposed opt-in, read-only, uv pip-only first step. The new flag, prototype, and narrower implementation refine that request rather than require a separate discussion. No previously merged general uv pip fix or subsequent regression was established.

The classification follows the existing request rather than the particular preview name or prototype. Both the inspection omission and additive-install consequences have longstanding open tracking issues. The canonical discussion already includes layered shared directories, heavy dependencies, read-only ownership, opt-in behavior, and a restricted `uv pip` first step. A more specific implementation proposal can be evaluated there.

The checkout confirms the discovery limitation. Documentation of that limitation is not the reason for classifying it as a duplicate; the decisive evidence is the matching open tracking discussion. The reporter's ABI-failure example, wheel counts, performance measurements, prototype correctness, and cross-platform coverage were not independently verified. No regression from a released general discovery fix was established.

## Related

- astral-sh/uv#2500 — issue, open: `uv pip list` does not read all system packages in `sys.path` and ignores `system-site-packages`. Canonical discussion for discovery beyond purelib/platlib, including layered environments with heavy dependencies. Its August 2025 comments already propose opt-in, read-only support limited to uv pip inspection and additive installs. The proposed full sys.path discovery and version shadowing differ from konstin's preference for explicit paths and failure on mismatches.
- astral-sh/uv#4466 — issue, open: UV upgrades packages inherited with --system-site-packages. Same install symptom: uv installs a newer environment-local package although an inherited version satisfies the request. Maintainers explicitly connect it to astral-sh/uv#2500; later comments describe read-only base environments and HPC-provided torch.
- astral-sh/uv#15162 — issue, open: Sharing torch between different projects. Direct match for the PYTHONPATH trigger: pip accepts an existing shared torch, while uv pip install downloads another copy. Maintainers discuss ownership and incompatible versions and refer to astral-sh/uv#11670.
- astral-sh/uv#12800 — issue, open: Recommended way to install additional packages in existing unmanaged environment. Requests additive installation over a Docker base environment without reinstalling torch. The reporter identifies --system-site-packages being ignored, and konstin explicitly identifies astral-sh/uv#2500 as the tracking issue.
- astral-sh/uv#9849 — pull request, open: Use `sys.path` for package discovery. Open draft explicitly targeting astral-sh/uv#2500 and astral-sh/uv#4466 by replacing scheme-only discovery with sys.path. Read-only uninstall handling remains a TODO; comments discuss stale interpreter paths after environment creation and additional platform and sync complications.
- astral-sh/uv#11670 — pull request, closed: Consider system site packages if activated. Closed without merge on June 23, 2026. Its described model closely matches ordered runtime discovery, read-only inherited distributions, satisfied requirements, and rejection of replacements that cannot win at runtime, but extends into sync, project, tool, and build environments.
- astral-sh/uv#15121 — pull request, merged: Include all site packages directories in ephemeral environment overlays. Merged August 8, 2025 for uv run --with overlay construction when a base interpreter has additional site directories. This is an adjacent prerequisite, not a general uv pip discovery fix, so it does not establish a regression in the new report.
- astral-sh/uv#18510 — issue, open: Interpreter cache pollution by system-site-packages venvs. Adjacent cache correctness issue: recreating a venv without system-site-packages can retain inherited directories in uv run --with overlays. A maintainer identifies the inclusion setting as relevant to the cache key. It informs caching design but does not establish the cause of the reported discovery omission.
- astral-sh/uv#21960 — pull request, closed: Retain base site-packages when creating virtual environments. Unmerged attempt to retain base site-package paths in interpreter metadata for newly created environments. Konstin's closing comment says caching must come first. This concerns metadata propagation rather than the proposed per-command uv pip discovery.

## Supporting evidence

- Current source: `crates/uv-cli/src/lib.rs:3001` documents that `--system-site-packages` affects runtime imports but does not make `uv pip list` or `uv pip install` account for inherited packages.
- Current source: `crates/uv-pip-commands/src/list.rs:95` builds `SitePackages` from the environment; `crates/uv-pip-commands/src/install.rs:331` uses the same environment index for satisfaction checks. `crates/uv-installer/src/site_packages.rs:80` iterates `interpreter.site_packages()`, and `crates/uv-python-interpreter/src/interpreter.rs:586` derives those paths from the target, prefix, or environment's `purelib`/`platlib`, rather than the full runtime path.
- In astral-sh/uv#2500, ncoghlan's June 25, 2024 comment describes `sitecustomize.py` layers, duplicate `torch` and CUDA packages, and unexpectedly large upper-layer archives. Their August 27, 2024 comment outlines satisfied inherited requirements, local upgrades without modifying the base, and refusal to remove inherited-only packages.
- In astral-sh/uv#2500 on August 19, 2025, konstin favors limited read-only support using specific paths, failing on incompatible requirements. Danielhollas proposes focusing on inspection and additive `uv pip` commands while leaving sync and uninstall aside. Ncoghlan discusses an opt-in flag on August 21. These comments support centralizing the proposal there, but do not establish agreement on full runtime discovery, shadowing, or the new uninstall behavior.
- In astral-sh/uv#4466, charliermarsh and zanieb explicitly identify the same underlying issue as astral-sh/uv#2500. Frazane's HPC example is recorded under the username `frazane` on April 10, 2026: users layer extra dependencies over site-provided `torch` and use pip to obtain the desired behavior.
- In astral-sh/uv#12800 on April 10, 2025, konstin explicitly identifies astral-sh/uv#2500 as the tracking issue after the reporter identifies ignored inherited site packages.
- astral-sh/uv#9849 is an open draft, last updated April 14, 2025. Its March 19, 2025 maintainer comment lists deduplication, Fedora paths, and non-uninstallable packages in sync among remaining complications. Its earlier comments identify interpreter-path staleness after deriving a newly created environment.
- astral-sh/uv#11670 has no merge timestamp and closed on June 23, 2026. Its current description includes fresh ordered runtime discovery, read-only effective-versus-owned distributions, precedence checks, and uncached runtime queries, with substantially broader command coverage.
- astral-sh/uv#15121 merged on August 8, 2025 and concerns ephemeral overlay construction for `uv run --with`; its linked discussion astral-sh/uv#15113 concerns copied entry points that cannot import OS packages on Debian/Fedora. This is distinct from installed-distribution indexing for `uv pip`.
- astral-sh/uv#21960 closed without merging; konstin's September 24, 2026 comment says, “Doesn't make sense without doing caching first.” This establishes a dependency concern for that metadata approach, not proof that a per-command query resolves every design problem.
- Followed the earlier attempt astral-sh/uv#2636, which targeted the same discovery issue and closed unmerged as stale, and merged astral-sh/uv#3500, which added interpreter `sys_path` metadata without implementing general installed-package discovery.

## Design decisions to carry into the canonical discussion

The proposed scope is narrower than astral-sh/uv#11670, but several choices remain: whether to trust all runtime paths or explicit read-only roots; whether mismatches should permit local shadows or fail; how import precedence and package-created `.pth` changes are handled; and how exact installation and uninstall distinguish effective distributions from mutable owned copies. The earlier comments are exploratory guidance, not approval of a feature name, implementation, or release plan.

The cache issue astral-sh/uv#18510 is relevant to the design but is not a confirmed cause of the omitted inherited packages in this report. Querying paths afresh is the reporter's proposed strategy, not a verified resolution of all cache and precedence concerns.

## Search coverage and exclusions

Decomposed inspection omissions, redundant additive installs, read-only ownership/uninstall, import precedence, and caching separately. Authenticated gh searches covered open and closed issues and attempted open, closed, and merged PR searches for sys.path, system-site-packages, PYTHONPATH/install/torch, pip list/system, inherited packages, layered environments, .pth/install, read-only packages, sys-path-discovery, and Cannot install/precedes; broadened beyond package and platform names. Inspected candidate bodies, comments, linked canonical discussions, and historical PRs. Direct PR searches returned no matches; linked PRs were inspected individually. Some later exact-warning and fix-oriented API searches hit GitHub's rate limit. Ruled out astral-sh/uv#3341 after comments identified in-environment .egg-info discovery, fixed by merged astral-sh/uv#3380; this is not inherited-path discovery. Also distinguished astral-sh/uv#22586 (runtime isolation in the Docker guide) and astral-sh/uv#17840 (project-interface imports). Astral-sh/uv#18062 shares the PyTorch Docker motivation but primarily requests uv sync support, excluded here. Source inspection confirmed scheme-based discovery; prototype behavior and benchmarks were not independently verified.

The strongest candidates were inspected with their comments, and reference chains were followed to the canonical discussion and historical fixes. No reproduction, build, or integration tests were run for this read-only triage. Repository and GitHub contents were not modified; this handoff and the structured result are the only authored artifacts, both under the runner's temporary directory.
