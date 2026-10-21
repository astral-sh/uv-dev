# Rebuild 0.13.4 Docker images to pick up patched ca-certificates (eMudhra CA compromise)

Issue: astral-sh/uv#22789

Classification: bug

## Summary

The reporter requests rebuilding the 0.13.4 derived Docker images, or publishing 0.13.5, to pick up patched system CA certificates after the eMudhra/emSign compromise. The concrete example is ghcr.io/astral-sh/uv:0.13.4-python3.11-trixie-slim: the reported dpkg-query output shows ca-certificates 20250419, while the upstream python:3.11-slim-trixie image reportedly contains 20250419+deb13u1.

The report distinguishes uv's bundled roots from the system trust store used by tools such as curl, git, and Python ssl. It suggests other Debian-derived variants may be affected, but supplies package output only for the Python 3.11 slim Trixie variant.

The closest work is the Alpine-only certificate update in astral-sh/uv#22767. Merged uv certificate fixes and the 0.13.5 version bump do not establish remediation of Debian image system certificates; no duplicate tracking that rebuild was found.

## Draft response

Your package-version comparison identifies a separate gap from the uv 0.13.4 fix: derived images inherit their system certificates from the base image, while astral-sh/uv#22760 updated uv's bundled roots. The Alpine update in astral-sh/uv#22767 does not cover Debian. The 0.13.5 version bump has merged in astral-sh/uv#22787, but refreshed image publication is not yet confirmed.

The next step is to verify the affected Debian variants and rebuild against patched bases, checking the resulting certificate packages on both architectures. In the meantime, you can use a freshly pulled, patched upstream Python image and copy /uv and /uvx from ghcr.io/astral-sh/uv:0.13.4, following the documented Docker installation pattern.

## Classification

The reported published image retains a system trust package containing an affected CA after uv's bundled-root fix. The Docker workflow confirms that derived images inherit upstream system packages without upgrading them; the advisory confirms that system trust stores require separate remediation. This is a security defect in a distributed image, not a request for new functionality. Image contents and broader variant impact remain to be independently verified. No existing item found tracks the Debian rebuild, and this is not a regression of the bundled-root fix.

The request is to correct the trust contents of an existing distributed image. The fact that upstream base-image publication follows a separate schedule explains how this can occur; it does not resolve the reported unsafe trust. The open Alpine patch is relevant parallel work, but its implementation explicitly excludes the Debian image family in this report.

## Related

- astral-sh/uv#22767 — Update CA certificates in Alpine Docker images (open, draft). Closest parallel remediation for the same CA incident. This draft explicitly upgrades Alpine certificate packages; its Dockerfile condition excludes Debian images, so it does not track the requested Trixie rebuild.
- astral-sh/uv#22760 — Bump webpki-root-certs to 1.0.10 (merged). The certificate fix shipped in uv 0.13.4 removes emSign ECC Root CA - G3 from uv's bundled roots. It changes only Cargo.lock and does not update the Docker base image's system certificate package.
- astral-sh/uv#22777 — Update rustls-webpki after the eMudhra incident (merged). Adds rejection of compromised intermediate public keys in uv's Rust certificate verifier. This further addresses the same incident but does not patch the system trust store used by other programs in derived images.
- astral-sh/uv#22773 — Stop updating EOL Python 3.9 and 3.10 docker images (merged). Explicitly cites compromised-CA fixes as a reason to stop publishing variants whose upstream images no longer receive security updates. It concerns retired Python 3.9/3.10 tags, whereas this report concerns supported Python 3.11/Trixie images awaiting a refresh.
- astral-sh/uv#22787 — Bump version to 0.13.5 (merged). The version bump requested as an alternative had already merged before this issue opened. The release API still lists 0.13.4 as latest and returns 404 for 0.13.5; the merged bump alone does not establish that refreshed Docker images were published.

## Supporting evidence

### Reported behavior and timing

The issue was opened on 2026-10-21 at 08:21:14 UTC and has no comments. The reporter says the uv image was built on 2026-10-20 at 14:51 UTC using a Python base from 2026-10-06, and that upstream rebuilt its Python image on 2026-10-21 at 01:09 UTC. These image timestamps and installed package versions are reporter-supplied evidence, not independently verified registry or container measurements.

The two reported commands are:

```sh
docker run --rm ghcr.io/astral-sh/uv:0.13.4-python3.11-trixie-slim dpkg-query -W ca-certificates
docker run --rm python:3.11-slim-trixie dpkg-query -W ca-certificates
```

The expected result is that newly published affected images contain the patched certificate package and generated trust bundle. The report concerns continued trust of compromised certificate chains, not a TLS connection failure or a package-resolution error.

### Repository-confirmed mechanism

In .github/workflows/build-docker.yml, the image matrix at lines 119–135 includes Debian, buildpack-deps, and Python Trixie bases. The generated Dockerfile at lines 306–313 uses FROM with the upstream image and copies uv and uvx into it; it does not run a system package upgrade. Updating the Rust binary therefore does not replace the inherited operating-system certificate bundle. The workflow builds linux/amd64 and linux/arm64 images.

docs/guides/integration/docker.md documents the distinction between binary-only distroless images and derived images, and shows copying uv into a chosen upstream Python image. docs/concepts/authentication/certificates.md documents bundled Mozilla roots as uv's default, with system and custom stores selected separately.

The published advisory GHSA-hq85-5cpj-mr62 states that 0.13.4 removes emSign ECC Root CA - G3 from uv's vendored bundle and that system/custom trust stores must be checked separately. The 0.13.4 GitHub release was published on 2026-10-20 at 15:24:59 UTC. Advisory: https://github.com/astral-sh/uv/security/advisories/GHSA-hq85-5cpj-mr62

The diff for astral-sh/uv#22767 adds certificate package upgrades only when the base is Alpine or Python-on-Alpine. Its description records an architecture-specific package-availability condition; that condition has not been independently rechecked. Nothing in that patch refreshes Debian packages or republishes the reported tag.

### Release-status caveat

astral-sh/uv#22787 merged on 2026-10-21 at 02:55:36 UTC, before this issue opened. The checkout's changelog includes 0.13.5, but the GitHub release API returns 404 for that tag and lists 0.13.4 as the latest published release at inspection. A merged version bump and changelog entry do not prove publication or patched image contents.

The changelog also describes a managed-Python update from astral-sh/uv#22786. Its metadata change was subsequently reverted by astral-sh/uv#22788 because of an interpreter crash; the checkout again references python-build-standalone 20261013. Neither change upgrades Debian's system ca-certificates package. Do not use the managed-Python changelog paragraph as evidence that the reported Docker image is fixed.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed, and merged PRs using authenticated gh. Separated stale system certificates, Docker rebuild/publication, and possible additional Debian variants. Literal queries included GHSA-hq85-5cpj-mr62, eMudhra/emSign, ca-certificates, 20250419, Trixie, and 0.13.4; conceptual queries covered root certificates, vulnerable/outdated base images, security updates, rebuild/refresh/republish, and internal:releases. PR searches returned no hits even for known matches, so supplemented them with the latest 1,000 PRs across all states, changelog references, and direct inspection of candidate bodies, comments, and relevant diffs. Ruled out astral-sh/uv#19236 and its fix astral-sh/uv#19253 (an unrelated CRL advisory), astral-sh/uv#15262 (missing image tags), and astral-sh/uv#12462 (a new Chainguard variant). The Alpine-only scope of astral-sh/uv#22767 rules it out as a duplicate.

Additional inspected issues concerned application dependency caching (astral-sh/uv#13727) and requests/certifi's choice of certificate bundle (astral-sh/uv#12871). Their maintainer comments identify different behavior from stale system certificates in published images. No canonical Debian rebuild discussion was found. Search results are not an exhaustive guarantee; direct PR listing was needed because PR search did not return known matching changes.

## Recommended next steps

1. Inspect current image digests and system certificate contents for the reported variant and the other supported Debian-derived variants on amd64 and arm64. Establish which upstream bases contain the patched bundles.
2. Determine whether refreshed images are already being published with 0.13.5 or whether a separate rebuild of affected tags is needed. Verify resulting image contents before marking this report resolved.
3. Keep the Alpine remediation coordinated with this work while tracking Debian verification separately. The documented copy-from-distroless pattern permits users to combine uv 0.13.4 with an independently updated Python base in the meantime.

Only this handoff README was authored. No checkout files were edited, no containers or builds were run, and no GitHub state was changed.
