# Different packages for same module: conflict resolution and uv remove problems

Issue: astral-sh/uv#22186

Classification: duplicate

## Summary

astral-sh/uv#15238 and astral-sh/uv#19412 track the removal failure; astral-sh/uv#15357 and astral-sh/uv#16546 cover collision detection. The proposed uninstall fix was closed unmerged, and collision warnings remain limited and opt-in.

The reporter uses uv 0.12.23 with Python 3.13.7 on Linux x86_64. They first add xgboost 3.0.5, then add xgboost-cpu 3.4.1 in a separate operation. Both distribution metadata directories remain, while importing xgboost reports the second distribution's version. Removing xgboost deletes most of the shared xgboost module directory but leaves xgboost_cpu-3.4.1.dist-info. A subsequent import raises `AttributeError: module 'xgboost' has no attribute '__version__'`. The reporter requests both conflict detection and protection of files needed by the surviving distribution.

## Draft response

This matches the shared-file removal problem tracked in astral-sh/uv#15238; astral-sh/uv#19412 shows the same uv remove/missing __version__ sequence with OpenCV. Uninstalling one distribution deletes its recorded files even when another distribution also supplies them, while the remaining metadata makes that package appear installed.

Different distribution names can resolve successfully despite overlapping module files. Collision detection is tracked in astral-sh/uv#15357 and astral-sh/uv#16546. The preview warning only compares packages installed in the same operation, so it does not cover your two separate uv add calls or prevent removal damage.

After removing xgboost, run `uv sync --reinstall-package xgboost-cpu` to restore the remaining package, and keep only one variant installed. Let's continue the removal discussion in astral-sh/uv#15238 and the collision-detection discussion in astral-sh/uv#15357.

## Classification

The incorrect removal behavior is already tracked by open astral-sh/uv#15238 and the nearly identical uv remove reproduction in astral-sh/uv#19412. The collision-detection request and sequential-add limitation are covered by astral-sh/uv#15357 and astral-sh/uv#16546. Duplicate takes precedence over bug because both claims can be centralized there. This is not an established regression: astral-sh/uv#19430 never merged, and the merged warning changes did not protect shared files during uninstall.

This is a correctness problem, not merely a support question. The source confirms that wheel uninstallation follows the removed distribution's RECORD without consulting the other distributions' RECORD files. The older reports establish the same mechanism and resulting incomplete environment. The warning implementation and its later refinements address diagnostics, not safe coexistence or repair. No evidence was found that shared-file removal was fixed and subsequently regressed.

## Related

- astral-sh/uv#15238 — `uv sync` removes necessary dependencies when two packages include the same module (issue, open). Canonical removal bug: a maintainer confirmed that uninstalling an overlapping distribution deletes files listed in its RECORD while the surviving distribution's dist-info remains. The discussion explicitly requests preserving shared files or detecting incomplete installations and recommends targeted reinstallation.
- astral-sh/uv#19412 — `uv sync` does not properly recover from an environment with faulty packages (opencv-contrib-python package problem) (issue, open). Nearly identical command sequence: install two differently named distributions providing cv2, remove one with uv remove, then get an AttributeError for the missing __version__ despite uv sync. The package names and platform differ, but the removal failure matches.
- astral-sh/uv#15357 — Stabilize conflicting modules warning (issue, open). Canonical tracker for diagnosing overlapping modules and their broken installations/uninstallations. It links astral-sh/uv#15238 and explains why warnings remain in preview; maintainers redirected the proposed shared-file preservation fix here.
- astral-sh/uv#16546 — report an error when installing a package that overrides another (issue, open). Matches the requested conflict error and sequential uv add trigger. Discussion confirms that the preview warning only detects packages installed together, misses a newly added package overlapping an already-installed one, and currently warns rather than rejects installation.
- astral-sh/uv#19430 — Preserve files claimed by another package's RECORD on uninstall (pull request, closed). Proposed protecting files claimed by other installed distributions, directly addressing astral-sh/uv#15238 and astral-sh/uv#19412. Closed without merging on 2026-05-17 in favor of astral-sh/uv#15357: maintainers noted that retained files could contain the wrong distribution's bytes. This was not a released fix.
- astral-sh/uv#13437 — Warn when two packages write to the same module (pull request, merged). Introduced collision warnings, merged 2025-08-08. Its description explicitly limits detection to packages installed in a single operation, explaining why separate uv add calls are not covered; it did not fix shared-file removal.
- astral-sh/uv#15253 — Move warnings for conflicting modules into preview (pull request, merged). Moved collision warnings behind preview on 2025-08-13 because of false positives. This explains the absence of a default warning and is not an uninstall fix that regressed in uv 0.12.23.
- astral-sh/uv#17623 — Better detection for conflicting packages (pull request, merged). Refined preview detection on 2026-01-21 to inspect overlapping paths and reduce namespace-package false positives. Current source still checks only the installation batch and uses file sizes as a heuristic; this does not prevent the reported removal damage.

## Supporting evidence

### Distinct claims and triggering conditions

1. Collision detection: `uv add xgboost==3.0.5` followed by `uv add xgboost-cpu` permits two distribution names to supply the same import module. The request is to detect or reject that overlap. Distribution identity, import-module identity, and filesystem path collisions are distinct concepts; sharing a directory alone is also valid for namespace packages.
2. Removal correctness: `uv remove xgboost` removes overlapping module files required by the retained xgboost-cpu distribution. Its metadata survives, so subsequent synchronization does not restore those files without forced reinstallation. The exact observable failure is the missing `xgboost.__version__`.
3. Recovery: after removing the unwanted variant, `uv sync --reinstall-package xgboost-cpu` is the applicable targeted repair. Avoid co-installing the overlapping variants. This follows the maintainer workaround in astral-sh/uv#15238; it is not a claim that overlap handling has been fixed.

### Source and existing test evidence

Source was inspected at checkout commit `01b62808962d7abfe2d10f43d652f357d8038202`.

- `crates/uv-install-wheel/src/uninstall.rs:28` reads only the distribution being removed's RECORD. The loop at line 49 deletes recorded paths, with removal at line 81, without cross-checking ownership against other installed distributions.
- `crates/uv-installer/src/satisfies.rs:88` checks registry requirements against installed distribution metadata and versions; its satisfaction path does not validate every recorded module file. This is consistent with the retained dist-info making the surviving distribution appear installed.
- `crates/uv-installer/src/installer.rs:183` creates a fresh InstallState for the current wheel installation batch and invokes conflict checking after those installations at line 216.
- `crates/uv-install-wheel/src/linker.rs:37` initializes the path map empty. Conflict warnings at line 106 are gated by DetectModuleConflicts. The comparison uses file sizes as a heuristic, so the feature is not a complete integrity or ownership check.
- `crates/uv/tests/pip_install/pip_install.rs:17246`, `overlapping_packages_warning`, already checks that a joint installation does not warn by default and does warn with `--preview-features detect-module-conflicts`. This test was read, not executed.

These findings establish the general behavior. The reporter's exact xgboost wheels were not downloaded or independently reproduced, and no builds or tests were run for this read-only triage.

### Maintainer decisions and release context

In astral-sh/uv#15238, a maintainer confirms that uninstalling types-boto3-full removes the module supplied by types-boto3-ecr while retaining the latter's dist-info, and suggests targeted reinstallation. astral-sh/uv#19412 reproduces the same command sequence and missing __version__ error with OpenCV on uv 0.11.14.

The warning was introduced by astral-sh/uv#13437 in August 2025, moved into preview by astral-sh/uv#15253 later that month, and refined by astral-sh/uv#17623 in January 2026. All precede this uv 0.12.23 report. They do not promise safe uninstallation or detection across separate installation operations.

The proposed ownership-based preservation in astral-sh/uv#19430 was closed without merging in May 2026. A maintainer explains that preserving a multiply claimed file does not establish that its bytes belong to the retained distribution, and redirects the work to astral-sh/uv#15357. The closed duplicate astral-sh/uv#20907 independently confirms that maintainers route collision diagnostics to that tracker and shared-file removal to astral-sh/uv#15238.

## Other candidates considered

The reporter's astral-sh/uv#16468 concerns removing and re-adding a single distribution on macOS, with a missing RECORD; a maintainer could not reproduce it. Its cause was not established as overlapping distributions, so it is not the canonical duplicate.

The reporter's astral-sh/uv#21968 concerns an absent NCCL library with intact metadata and synchronization that does not repair it. File loss there is explicitly unexplained. It is adjacent to the incomplete-environment symptom, but does not establish the same removal trigger.

astral-sh/uv#10564 and astral-sh/uv#7389 discuss detecting file or import-module collisions more broadly. astral-sh/uv#17645 shows the same removal mechanism when switching typer/typer-slim extras; its maintainer discussion clarifies that retaining files can still leave incorrect contents. The selected items give more direct coverage of the reported command sequence and current maintainer direction.

## Search scope and limitations

Separated collision detection/rejection during uv add from shared-file deletion and failed repair after uv remove. Searched astral-sh/uv open and closed issues with literal xgboost-cpu, the exact xgboost AttributeError, and generalized AttributeError/__version__/remove terms; conceptual searches covered same module, overlapping files, shared files, file collisions, and RECORD/uninstall. Searched open, closed, and merged PRs for conflicting modules, uninstall/shared, collision, module, overlap, and conflict, and followed issue/comment references to inspect PRs and their review discussions. REST searches became rate-limited; continued with authenticated gh issue/pr list searches. PR search queries returned no hits, so the relevant PRs were verified through direct gh pr view calls. Reviewed warning introduction, preview rollback, detection refinement, and the unmerged preservation proposal for release/regression context. Inspected astral-sh/uv#10564, astral-sh/uv#7389, astral-sh/uv#17645, and the duplicate-routing comments in astral-sh/uv#20907. Ruled out the reporter's astral-sh/uv#16468 as the canonical match: it concerns remove/re-add of one distribution with a missing RECORD and no confirmed overlap. astral-sh/uv#21968 shares incomplete-environment acceptance, but its file-loss cause is unknown, unlike this explicit overlapping-distribution removal.

All GitHub operations were read-only. The reply above is a draft for maintainer review. The only requested handoff change is this README; no checkout files or GitHub resources were changed.
