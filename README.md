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

The repository currently provides related but narrower safeguards. `uv lock --check` checks whether
the lockfile is up to date with project metadata; it is not an independent provenance check for all
lockfile records. During sync, uv derives a hash-verification strategy from the lock resolution and
rejects downloaded bytes that do not match hashes recorded in `uv.lock`. Consequently, changing
only an artifact URL cannot substitute different content without a hash mismatch, although it can
still redirect the request, and changing both the URL and trusted hash remains outside that
protection. The closest current CI workflow is `uv lock --refresh` followed by
`git diff --exit-code -- uv.lock`, which revalidates index metadata and exposes any regenerated
lockfile diff.

No existing issue or pull request covers the complete network-backed provenance comparison.
astral-sh/uv#11932 discusses a broader check command and the boundary between project, lockfile,
environment, and hash checks. astral-sh/uv#12276 directly tracks the requested version/constraint
subset, and astral-sh/uv#12235 implemented one narrower structural consistency check for package and
wheel versions.

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

## Draft response

`uv lock --check` currently checks whether `uv.lock` is consistent with the project metadata; it
does not independently authenticate every locked dependency and artifact URL against freshly
fetched index metadata. During sync, uv does verify downloaded artifacts against the hashes recorded
in `uv.lock`, so changing only a wheel URL would not allow different bytes to be installed without a
hash mismatch. However, the lockfile remains the source of truth for the URL and hash pair, so
independently validating both would be a new capability.

The closest CI check today is to run `uv lock --refresh` and then
`git diff --exit-code -- uv.lock`, which forces index metadata revalidation and reports whether uv
would regenerate the lockfile differently. The narrower version/constraint validation is already
tracked in astral-sh/uv#12276, and astral-sh/uv#11932 discusses broader check semantics, but neither
covers the full provenance check requested here. We can keep this issue open to track a dedicated
verification workflow and its exact guarantees.

## Classification

This is an enhancement. The report asks for a new, independently network-backed verification mode
and does not demonstrate a violation of a guarantee currently made by `uv lock --check`. Existing
behavior checks project/lock freshness and verifies downloaded artifacts against lockfile hashes,
but the requested trust model requires using configured-index metadata as an independent source for
the dependency graph, URLs, and hashes.

The issue is not a duplicate. Existing discussions cover important subsets, but none covers the
full capability. In particular, astral-sh/uv#12276 does not fetch index metadata or validate artifact
provenance, and astral-sh/uv#11932 primarily asks to compare a project environment with its lockfile.

## Related

- astral-sh/uv#11932 — **`uv lock --check` doesn't error if environment doesn't match lockfile**
  (open issue). This is the closest broad command-design discussion. It asks for a check across the
  project, lockfile, environment, and hashes. A maintainer clarifies that `uv lock --check` checks
  project/lock consistency and that validating artifact hashes themselves requires accessing the
  artifacts. It does not ask to prove that locked URLs and hashes were advertised by an index.
- astral-sh/uv#12276 — **Validate locked versions against constraints in lock file** (open issue).
  This directly matches one requested check: rejecting a manually corrupted lockfile whose selected
  versions violate its recorded project constraints. Maintainers welcomed additional validation,
  but the issue does not cover dependency provenance or comparison with fresh index metadata.
- astral-sh/uv#12235 — **Error on lockfiles with incoherent wheel versions** (merged pull request).
  This added a narrower structural integrity check after externally edited lockfiles paired package
  versions with inconsistent wheel versions. It demonstrates an existing approach to rejecting
  internally incoherent lock contents, but does not validate artifact URLs against an index.
- astral-sh/uv#18781 — **Reject locked malware installations** (closed issue). Maintainers explicitly
  discuss preserving index-free locked installs for performance and using OSV malware reports as a
  cheaper layered defense. They also confirm that a PyPI artifact referenced directly by a lockfile
  can remain retrievable after its index entry is quarantined or removed. This explains the current
  design tradeoff but does not validate arbitrary locked URLs or previously unknown malware.
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
resolution.
