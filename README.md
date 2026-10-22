# Git dependency checkout can race with detached `git maintenance` started by uv's own fetch (Git 2.54+)

Issue: astral-sh/uv#22829

Classification: bug

## Summary

The reporter observed two failed Git dependency installs in GitHub-hosted Ubuntu CI with uv 0.13.4 and Git 2.55.0. Installing twice from the same local `git+file://` URL caused a subsequent fetch into an existing Git cache database. Checkout creation failed with exit status 128 when `git clone --no-hardlinks` could not copy either a pack file or `multi-pack-index.lock`, reporting `No such file or directory`.

The reporter could not reproduce the failure locally in 81 installs. Successful Trace2 and syscall traces reportedly showed fetch starting detached maintenance, followed by a local clone overlapping repacking and deletion of a pack it had just linked. The causal link to the two failed CI runs remains inferred.

No duplicate found. astral-sh/uv#11420 and astral-sh/uv#11421 explain the hardlink fallback; astral-sh/uv#5051 addresses concurrent uv processes. None addresses detached Git maintenance during checkout.

## Draft response

The current code leaves automatic maintenance enabled during fetch, then locally clones the same database. The --no-hardlinks fallback from astral-sh/uv#11421 still copies files from that database, so it does not prevent concurrent repacking. This is consistent with the overlap you traced, although it does not confirm maintenance caused the two CI failures.

Please share the minimal script and maintenance settings used to capture that overlap; it need not reproduce the failure reliably. The next step is to test disabling automatic maintenance for uv's fetches and add regression coverage. Your per-process maintenance.auto=false and gc.auto=0 workaround is reasonable in the meantime.

## Classification

The current source permits automatic maintenance during fetch and then performs a local clone from the same database; Git documents that maintenance may detach and local cloning can race with concurrent source modification. Intermittent checkout failure is incorrect behavior, even without a deterministic reproduction. The reporter's successful overlap traces support the mechanism but do not prove it caused the two CI failures. No duplicate was found, and the historical hardlink and inter-process-locking fixes address different problems rather than a previously fixed maintenance race.

Git 2.54's change to geometric maintenance is verified in upstream release notes. It supplies a version-specific trigger for an existing unsafe interaction; there is no evidence that a previously shipped uv fix for this maintenance race has regressed. The suggested opt-out restores reliable checkout behavior, so this is a correctness issue rather than an enhancement or support question.

## Related

- astral-sh/uv#11421 — retry local clones without hardlinks if they fail (pull request, merged). Introduced the exact --local to --no-hardlinks fallback seen in this report. Merged on 2025-02-12 to fix astral-sh/uv#11420, with successful reporter validation. Its diff changes clone retry behavior only; copying still reads the same database and does not prevent concurrent maintenance.
- astral-sh/uv#11420 — installing git repository ignores UV_LINK_MODE, breaks on some filesystems (issue, closed). Same cache-database-to-checkout clone path, but the failure was 'Operation not permitted' on BeeGFS without hardlink support. Maintainers clarified that UV_LINK_MODE does not control these internal links and resolved it through astral-sh/uv#11421. Disappearing pack files during maintenance are a different failure.
- astral-sh/uv#5051 — fix(git): lock cache on resolve (pull request, merged). Fixed a confirmed Git-cache clone race between multiple uv processes resolving branch references, producing 'destination path ... already exists'. Merged on 2024-07-16 after maintainer reproduction and verification. The repository lock remains in current source; it coordinates uv operations, not Git's detached maintenance, so this report does not establish a regression of that fix.

## Report decomposition

- Observable symptoms: intermittent local clone failure; missing `objects/pack/pack-*.pack` or `objects/pack/multi-pack-index.lock`; `Git operation failed` and exit status 128.
- Subsystems and commands: Git dependency installation, `git-v1/db`, `git-v1/checkouts`, `fetch_with_cli`, `GitCheckout::clone_into`, `git fetch --force --update-head-ok`, `git clone --local`, and the `--no-hardlinks` retry.
- Triggering conditions: reused cache database, another fetch and new checkout after an earlier install, local file URL in the reported CI job, uv 0.13.4, and Git 2.55.0. Git 2.54 changed the default maintenance strategy.
- Proposed mechanism: fetch spawns `git maintenance run --auto --detach`; geometric repacking and multi-pack-index updates modify the source database during local cloning.
- Requested correction: suppress automatic maintenance for uv-owned fetches, or explicitly coordinate maintenance with cache use. The reporter suggests `--no-auto-gc` / `--no-auto-maintenance` or per-process `maintenance.auto=false` and `gc.auto=0`.

## Supporting evidence

Source inspected at checkout commit `e5cb58f6b0bd2795138fa831e1f5b5ce137ca200`:

- `crates/uv-git/src/git.rs:745`: `fetch_with_cli` builds a normal fetch with `--force` and `--update-head-ok`; it adds no automatic-maintenance opt-out. The shared Git process builder also adds no such setting.
- `crates/uv-git/src/source.rs:112` and `crates/uv-git/src/source.rs:149`: the fetch/update path is followed by `db.copy_to` when a checkout is needed.
- `crates/uv-git/src/git.rs:399`: checkout creation invokes `git clone --local`, then retries with `--no-hardlinks` on failure. Both commands read the same local database path.
- `crates/uv-git/src/resolver.rs:232`: an exclusive per-repository uv lock surrounds source fetching and checkout. This serializes cooperating uv operations; it does not make a detached Git maintenance child wait before the same operation starts cloning.
- astral-sh/uv#11421 changes only the local-clone fallback. Its comments record successful BeeGFS validation. astral-sh/uv#5051 routes reference resolution through the locked fetch path; maintainer comments confirm the original cross-process failure and the fix.
- `crates/uv/tests/pip_compile/pip_compile.rs:2483`: `compile_git_concurrent_access` covers two packages at the same pinned Git commit in one resolution and is gated with `test-git`. It does not force automatic maintenance or exercise the reported second-fetch overlap. A search of Rust integration tests found no explicit maintenance/auto-GC regression coverage.

Upstream Git sources independently checked:

- [Git 2.54 release notes](https://raw.githubusercontent.com/git/git/v2.54.0/Documentation/RelNotes/2.54.0.adoc): maintenance starts using the geometric strategy by default.
- [Git 2.55 maintenance configuration](https://raw.githubusercontent.com/git/git/v2.55.0/Documentation/config/maintenance.adoc): `maintenance.auto` defaults to true; `maintenance.autoDetach` falls back to `gc.autoDetach` and defaults to true.
- [Git 2.55 clone documentation](https://raw.githubusercontent.com/git/git/v2.55.0/Documentation/git-clone.adoc): local cloning can race with concurrent source modification; `--no-hardlinks` copies local object files instead of linking them.
- [Git 2.55 fetch options](https://raw.githubusercontent.com/git/git/v2.55.0/Documentation/fetch-options.adoc): automatic maintenance after fetch is enabled by default and can be disabled with `--no-auto-maintenance` or `--no-auto-gc`.
- [Git 2.23 fetch options](https://raw.githubusercontent.com/git/git/v2.23.0/Documentation/fetch-options.txt): `--[no-]auto-gc` was already available, supporting compatibility evaluation of the proposed flag.

These sources establish the exposure to concurrent mutation. The failing CI runs and successful overlap traces are reporter-provided evidence; no failing execution was independently reproduced.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separately searched missing-file symptoms ('failed to copy file', 'No such file or directory', 'multi-pack-index.lock', 'failed to create link', 'no-hardlinks'); proposed causes and remedies ('git maintenance', 'repack', 'garbage collection', 'maintenance.auto', 'maintenance.autoDetach', '--no-auto-gc', '--no-auto-maintenance', 'fetch_with_cli'); and conceptual/version variants (Git race, concurrent access, cache corruption, area:cache, Git 2.54/2.55), including closed-fix searches. PR keyword searches omitted even a known historical match, so additionally scanned all 13,224 PR titles through the paginated all-state API and inspected relevant bodies, comments, reviews, and diffs. Followed issue-to-fix and canonical-discussion links. Ruled out astral-sh/uv#10758 (directory-removal existence checks), astral-sh/uv#5645 (older checkout/index.lock failures without a confirmed matching mechanism), astral-sh/uv#6484 (DNS/ref-fetch errors and a missing restored repository), and astral-sh/uv#4378 with astral-sh/uv#4698 (incorrect commit-presence checks). Also distinguished CI revision-resolution failures and Windows/container cache-access discussions from disappearing Git pack files. No existing discussion or fix for the maintenance overlap was found.

Additional distinctions from inspected discussions:

- astral-sh/uv#10758 is a particularly plausible title match, “Remove TOCTOU errors in Git clone,” but its diff only replaces existence-before-removal checks with removal that tolerates missing directories. It neither changes fetch maintenance nor stabilizes pack enumeration.
- astral-sh/uv#14894 reports unresolved commit IDs in GitHub Actions. Its related closed, unmerged experiment, astral-sh/uv#14895, adds general Git retries without establishing this maintenance mechanism.
- astral-sh/uv#16467 concerns generated files removed by `git clean` in editable projects.
- astral-sh/uv#7434 concerns cache access across containers. Maintainers redirected its Windows reports to astral-sh/uv#11002, subsequently consolidated into astral-sh/uv#2810. Those reports concern temporary-file persistence, permissions, and wheel files rather than Git object packs.
- astral-sh/uv#1760 concerns Git cache space efficiency, without the reported checkout failure.
- The timeline of astral-sh/uv#22829 has no linked fixing PR at assessment time.

## Recommended next step and validation

Obtain the small script and maintenance-trigger settings used for the successful overlap trace. A deterministic reproduction of the CI failure is not required to investigate or classify this bug.

Evaluate disabling automatic maintenance specifically for uv's fetches, including Git-version compatibility. Add focused integration coverage for a fetch into a reused database followed by creation of a new checkout, with maintenance made eligible to run. Follow existing command snapshots and `test-git` gating; distinguish this scenario from the existing same-commit concurrency tests. If explicit maintenance is desired later, its lifetime must be coordinated with cloning, not merely started while a uv lock is held.

The documented per-process Git configuration is a reasonable temporary workaround. It removes the automatic-maintenance trigger described here; it is not evidence that every intermittent Git-cache error has that cause.

No builds or tests were run for this assessment. Validation consisted of source review, upstream documentation, and issue/PR history.
