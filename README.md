# `uv sync` validates a `build-system.requires` wheel against the wrong platform's hash from `uv.lock`

Issue: astral-sh/uv#21608

Classification: bug

## Summary

`uv sync --no-install-project` fails while preparing an editable dependency's isolated build
environment when a `[build-system].requires` package has the same name and version as a package in
the project lockfile but resolves to a different platform/index artifact. On Linux aarch64, the
reported reproduction downloads the intended wheel and computes its correct digest, then rejects it
against a digest belonging to another wheel. The same project works on Linux x86_64 and with uv
0.12.10, while uv 0.12.11 through 0.12.13 fail.

The version boundary aligns directly with astral-sh/uv#21223, merged for uv 0.12.11. That change
made `uv sync` derive its isolated-build hash policy from the full `uv.lock`; previously the build
hash policy was empty. The current lock-derived strategy keys registry hashes by package name and
version, while an isolated build resolution can select an artifact from a different source or
platform for that same identity. This source history establishes a regression in the relevant path,
although the attached reproduction should still be converted into an integration test to pin down
the minimal combination of markers and indices.

No existing open issue or pull request tracks this specific regression. There are two close
historical precedents in which hashes belonging to one artifact were applied to another artifact of
the same package: astral-sh/uv#7059 with platform-conditioned repeated requirements, and
astral-sh/uv#13112 with wheel-versus-sdist hashes.

## Draft response

Thanks for the focused reproduction. This is a bug, and the uv 0.12.11 boundary matches
astral-sh/uv#21223, which changed `uv sync` to verify isolated build dependencies using hashes
derived from the full `uv.lock`. In this case, the build resolver can select a different
platform/index artifact with the same package name and version, but that artifact should not be
rejected against hashes belonging to the project's other locked artifact.

This is the same class of artifact-scoping problem addressed historically by astral-sh/uv#7060 and
astral-sh/uv#17157, but neither existing discussion tracks this build-isolation regression, so this
issue should remain open. The next implementation step is to turn the provided case into an
integration regression test with distinct hashes for the platform-specific wheels, then scope the
build-dependency check to the relevant locked artifact/source without weakening the locked-source
validation added in astral-sh/uv#21223.

## Classification

This is a `bug`, not an enhancement or support question: a valid wheel is rejected as corrupt even
though its computed hash is correct. It is not a duplicate because no open issue or pull request
already tracks the regression in the isolated build-dependency path.

The regression timing is source-backed. astral-sh/uv#21223 shipped in uv 0.12.11 and changed project
sync from `HashStrategy::default()` for build dependencies to a strategy created from the full
lockfile. The implementation groups registry hashes under a name/version identity, independent of
the index and platform artifact selected by the isolated build resolver. This creates the relevant
opportunity for a build artifact that is not the project's selected locked artifact to inherit the
project artifact's expected hash.

The exact symptom has been fixed in adjacent paths before. astral-sh/uv#7060 made distribution
hashes take precedence over package-level registry hashes after astral-sh/uv#7059 mixed Linux and
macOS hashes for a repeated requirement. astral-sh/uv#17157 changed lockfile installation to return
only the selected wheel or sdist's hash after astral-sh/uv#13112 demonstrated a wheel being checked
against its package's sdist hash. Because the present report is a new trigger introduced after
those fixes, it is a regression bug rather than a duplicate of either closed issue.

## Related

- astral-sh/uv#21223 — **Verify locked source archives before building metadata** (merged). This is
  the closest source change: it landed in uv 0.12.11 and explicitly began verifying isolated build
  dependencies against hashes derived from the project lockfile.
- astral-sh/uv#7059 — **Wrong hash is used for repeated `tensorflow-text`** (closed). It reports the
  same observable failure: the correct macOS wheel hash was computed, but uv expected the Linux
  registry hash because the package appeared under multiple platform conditions. Its path was
  ordinary dependency resolution rather than isolated build dependency resolution.
- astral-sh/uv#7060 — **Use distribution hash over registry hash** (merged). It fixed
  astral-sh/uv#7059 by prioritizing hashes for the specific distribution over related package-level
  registry hashes.
- astral-sh/uv#13112 — **SHA mismatch if wheel has no sha in the index, but sdist has** (closed).
  This demonstrates another lockfile artifact-association failure, where uv checked a wheel against
  the same package/version's sdist hash. The trigger was a missing wheel hash, not a platform- and
  index-specific isolated build.
- astral-sh/uv#17157 — **Avoid enforcing incorrect hash in mixed-hash settings** (merged). It fixed
  astral-sh/uv#13112 by associating hashes with the selected wheel or sdist instead of concatenating
  hashes for the package.

## Search evidence

Literal searches covered `Hash mismatch`, `build-system.requires`, `uv sync
--no-install-project`, `uv.lock`, `aarch64`, platform-specific wheels, and the reported uv
0.12.10-to-0.12.11 boundary. Conceptual searches covered isolated build environments, build
dependencies, distribution-specific versus package-level hashes, repeated package identities,
conditional sources, multi-index resolution, and wrong-artifact verification. Open and closed
issues and open, closed, and merged pull requests were included, and the uv 0.12.10-to-0.12.11
release comparison was inspected for fix-oriented history.

Several plausible results were ruled out. astral-sh/uv#18938 and astral-sh/uv#16784 involved
missing hashes in the upstream PyTorch index and were resolved externally. astral-sh/uv#17260 was
an uncompressed-versus-zstd hash regression, not a platform/source collision. astral-sh/uv#14000
concerns choosing candidates before hash validation, while this report downloads the intended
artifact and rejects it afterward. astral-sh/uv#18059 concerns source propagation for
`extra-build-dependencies`, not lock-derived verification of a standard `build-system.requires`
dependency.
