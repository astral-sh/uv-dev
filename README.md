# Command to verify uv.lock is legitimate

Issue: astral-sh/uv#21484

Classification: enhancement

## Summary

The report requests a CI-friendly command that treats a proposed `uv.lock` as untrusted and
independently checks it against the project and freshly fetched package-index metadata. The desired
checks include confirming that the dependency graph is justified by `pyproject.toml`, selected
versions satisfy declared constraints, and recorded wheel and source-distribution URLs and hashes
are actually advertised by the configured indexes. The motivating case is a pull request that hides
an attacker-controlled artifact URL among otherwise legitimate lockfile changes.

The reporter clarifies that this is an explicit trust-boundary check for contributions from
non-maintainers, intended for pull-request CI or manual local review of a checked-out contribution.
They are not requesting an index-backed verification step on every `uv sync`.

The repository currently provides related but narrower safeguards. `uv lock --check` checks whether
the lockfile is up to date with project metadata; it is not an independent provenance check for all
lockfile records. During sync, uv derives a hash-verification strategy from the lock resolution and
rejects downloaded bytes that do not match hashes recorded in `uv.lock`. Consequently, changing
only an artifact URL cannot substitute different content without a hash mismatch, although it can
still redirect the request, and changing both the URL and trusted hash remains outside that
protection. The closest current CI workflow is `uv lock --refresh` followed by
`git diff --exit-code -- uv.lock`, which revalidates index metadata and exposes any regenerated
lockfile diff. A maintainer has now proposed `uv lock --check --refresh --no-build` as a direct,
non-writing form of this check. The reporter confirms that this is the behavior they were seeking.
Source inspection supports its intended mechanics, though the exact tampering reproduction has not
been independently run with that command in this handoff environment.

The core capability therefore appears to exist as a composition of current flags. The remaining
request is discoverability: documenting the secure combination prominently enough that reviewers
know to opt into it. astral-sh/uv#11932 discusses a broader check command and the boundary between
project, lockfile, environment, and hash checks. astral-sh/uv#12276 directly tracks the requested
version/constraint subset and already contains a request to document a refresh-based CI pattern;
astral-sh/uv#12235 implemented one narrower structural consistency check for package and wheel
versions.

The follow-up comment adds a concrete report against uv 0.12.9. After generating a project locked
with `six`, changing only `files.pythonhosted.org` to the lookalike
`files.pythonhcsted.org` leaves `uv lock --check` successful. A subsequent uncached locked sync
attempts to resolve the modified host and fails at DNS lookup. The commenter also reports that
adding an otherwise unjustified registry package to the lockfile and linking it from the root
package's resolved dependency list still passes `uv lock --check` when the project metadata is left
unchanged.

## Reproduction

The new comment provides these macOS-style shell steps, reported with uv 0.12.9:

```console
$ uv init --name demo .
$ uv add six
$ sed -i '' 's#files.pythonhosted.org#files.pythonhcsted.org#' uv.lock
$ uv lock --check
# exits 0
$ rm -rf .venv
$ uv sync --locked --no-cache
# fails resolving files.pythonhcsted.org
```

This has not been independently executed in the handoff environment because no `uv` executable is
installed there. The checkout source does support the mechanism described by the commenter:
`Lock::satisfies` treats registry and Git sources as immutable and skips per-package metadata and
dependency validation for them. When the overall satisfaction check succeeds, the lock operation
can return the existing lock unchanged. Separately, the sync path builds its download hash policy
from the accepted lock resolution, so the changed URL is consulted before downloaded content can be
checked against the lockfile hash.

## Verification command

A maintainer proposed the following existing flag combination, and the reporter confirms it is the
behavior they wanted:

```console
$ uv lock --check --refresh --no-build
```

Source inspection indicates that it should detect both reported mutations:

- `--refresh` prevents the existing lock from taking the normal satisfied/unchanged fast path and
  performs a new resolution against refreshed metadata while retaining locked versions as
  preferences where possible.
- `--check` runs that lock operation without writing `uv.lock` and returns a lock-mismatch error if
  the newly produced lock differs from the existing one. A substituted artifact URL or an
  unjustified dependency should therefore make the check fail.
- `--no-build` prevents resolution or metadata validation from building source distributions. This
  is important when checking an untrusted contribution because package builds can execute code.

The tradeoff is that `--no-build` can reject an otherwise legitimate project when a dependency or
dynamic local project exposes metadata only through a build. The lock implementation deliberately
propagates that disabled-build error rather than falling back, so the proposed command fails closed
instead of executing build code. The command is source-supported and accepted by the reporter as
the desired workflow, but has not yet been independently exercised against the uv 0.12.9
reproduction here.

The reporter's remaining concern is visibility. They report spending substantial time trying
insufficient combinations such as plain `uv lock --check`, and suggest documenting the secure
workflow on both the locking-and-syncing page and the GitHub Actions integration page. They offered
to contribute that documentation if maintainers want it. A maintainer cautioned that documentation
updates are generally difficult for external contributors because the project evaluates them
against the broader product picture. This is not an explicit rejection of documentation changes,
but it indicates that maintainers should first decide the desired guidance and placement rather
than treating the contributor's offer as pre-approved.

## Classification

This remains an enhancement, now best understood as a documentation and discoverability improvement
rather than necessarily a new command. Plain `uv lock --check` does not promise provenance
validation, while the existing `--check --refresh --no-build` combination appears to supply the
requested network-backed, non-writing, fail-closed workflow.

The reporter is open to treating this as a duplicate of astral-sh/uv#12276 because that discussion
also requests refresh-oriented documentation. However, astral-sh/uv#12276's tracked implementation
scope is narrower—validating locked versions against constraints—so a maintainer decision is still
needed on whether to centralize the documentation request there or retain this issue's distinct
untrusted-contribution security framing.

## Related

- astral-sh/uv#11932 — **`uv lock --check` doesn't error if environment doesn't match lockfile**
  (open issue). This is the closest broad command-design discussion. It asks for a check across the
  project, lockfile, environment, and hashes. A maintainer clarifies that `uv lock --check` checks
  project/lock consistency and that validating artifact hashes themselves requires accessing the
  artifacts. It does not ask to prove that locked URLs and hashes were advertised by an index.
- astral-sh/uv#12276 — **Validate locked versions against constraints in lock file** (open issue).
  This directly matches one requested check: rejecting a manually corrupted lockfile whose selected
  versions violate its recorded project constraints. Maintainers welcomed additional validation,
  and a later comment requests documentation for a refresh-based CI check. Its implementation scope
  does not cover dependency provenance, but it may serve as the canonical documentation discussion
  if maintainers choose to consolidate this issue there.
- astral-sh/uv#12235 — **Error on lockfiles with incoherent wheel versions** (merged pull request).
  This added a narrower structural integrity check after externally edited lockfiles paired package
  versions with inconsistent wheel versions. It demonstrates an existing approach to rejecting
  internally incoherent lock contents, but does not validate artifact URLs against an index.
- astral-sh/uv#18781 — **Reject locked malware installations** (closed issue). Maintainers explicitly
  discuss preserving index-free locked installs for performance and using OSV malware reports as a
  cheaper layered defense. They also confirm that a PyPI artifact referenced directly by a lockfile
  can remain retrievable after its index entry is quarantined or removed. This explains the current
  design tradeoff but addresses compromised dependencies during installation, not deliberate
  lockfile manipulation at contribution-review time. The requested verifier would be invoked
  separately rather than changing the normal locked-sync path.
- astral-sh/uv#18936 — **Reject locked malware installations** (merged pull request). This added a
  malware check against `MAL-` OSV reports before project installation. It checks known malicious
  package versions rather than establishing that the dependency graph, artifact URL, and hash came
  from the configured index.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
used `uv.lock`, `verify`, `legitimate`, `tamper`, `malicious`, and artifact URL terminology.
Conceptual and fix-oriented queries covered lockfile integrity and authenticity, project-lock
consistency, dependency constraints, hashes, index metadata, artifact provenance, refresh and
re-resolution, CI checks, and `uv audit`. The maintainer-linked chain through astral-sh/uv#12254 and
astral-sh/uv#12235 was also inspected.

Two plausible adjacent candidates were ruled out. astral-sh/uv#18619 concerns only a parser-side
missing-hash validation gap for direct/path source distributions, not validating registry metadata;
it was closed after a maintainer said the tampered-lockfile-only case was not worth fixing.
astral-sh/uv#18562 restates the internal version/constraint inconsistency already discussed in the
earlier, maintainer-engaged astral-sh/uv#12276 and likewise does not cover artifact provenance.
astral-sh/uv#18506 is the `uv audit` roadmap, but it concerns known vulnerability, malware, and
project-status advisories rather than authenticity of dependency graph or artifact provenance.

Repository evidence supports the distinction above: the locking documentation defines
`uv lock --check` as checking whether project metadata makes a lockfile outdated; cache
documentation defines `--refresh` as forcing cached package metadata to be revalidated; and the
project sync implementation constructs a verifying hash strategy from the lock resolution before
installing artifacts. The new source inspection further confirms that registry and Git sources are
classified as immutable by `Source::is_immutable`; `Lock::satisfies` skips metadata and dependency
validation for such packages, and a satisfied lock can be returned unchanged without a fresh
resolution. Conversely, the explicit `--refresh` path marks the existing lock as merely preferable,
constructs a new lock from the resulting resolution, and `--check` raises a mismatch when that lock
differs. Disabled-build errors are propagated when metadata cannot be obtained under `--no-build`,
which preserves the fail-closed property needed for reviewing untrusted lockfile changes.
