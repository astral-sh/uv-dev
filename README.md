# Rebuild 0.13.4 Docker images to pick up patched ca-certificates (eMudhra CA compromise)

Issue: astral-sh/uv#22789

Classification: bug

## Summary

The reported stale system certificate package is **reproducible** in the published `linux/amd64` image `ghcr.io/astral-sh/uv:0.13.4-python3.11-trixie-slim`. Its installed package database contains `ca-certificates 20250419`, and its generated system CA bundle still trusts `emSign ECC Root CA - G3`. The current upstream `python:3.11-slim-trixie` image contains `20250419+deb13u1` and excludes that root. Both images identify Python 3.11.17 and Debian 13 (Trixie).

This independently confirms the concrete package comparison in the report and retention of the affected root. uv 0.13.4 updated its bundled roots; that update does not replace the system trust store inherited by derived Docker images. Other Debian variants and `linux/arm64` remain unverified. No live connection using a compromised intermediate was attempted.

The closest parallel work is the Alpine-only certificate update in astral-sh/uv#22767. The 0.13.5 version bump has merged, but the release API still reports 0.13.4 as latest and returns 404 for 0.13.5. A version bump alone does not demonstrate refreshed image publication.

## Classification

This is a security defect in the contents of a distributed image, with a request to rebuild the affected images or publish a new release. The reported image demonstrably retains the affected root in its system trust bundle after uv updated its separately bundled roots. It is not evidence of a regression in uv’s bundled-root fix.

The Docker workflow inherits operating-system packages from upstream bases without upgrading them. Different publication times explain the discrepancy, and the current upstream Python image provides a patched comparison. The Alpine-only work does not cover this Debian variant. No duplicate tracking the Debian rebuild was identified in the context-gathering search.

## Reproduction

**Outcome: reproducible.** Inspected the public registry artifacts on 2026-10-21 using anonymous registry access, with all downloaded images, configuration, and analysis files under `/tmp/uv-22789.8F3orU`.

### Environment and scope

- Host: Linux x86_64; installed `uv --version`: `uv 0.13.4 (x86_64-unknown-linux-gnu)`.
- Inspection tools: Skopeo 1.13.3, host Python 3.12.3, host OpenSSL 3.0.13.
- Selected image platform: `linux/amd64`; both image configurations report Python 3.11.17, and their OS metadata identifies Debian GNU/Linux 13 (Trixie).
- No project, dependency groups, uv settings, or Python packages are needed: this concerns the published operating-system package database and trust bundle.
- Docker daemon access failed with permission denied on `/var/run/docker.sock`. Consequently, the reported `docker run` commands were not executed. Image layers were downloaded without a daemon and their package/trust files inspected directly.

### Minimal reported commands

On a host with Docker access, the report’s package comparison is:

```sh
docker run --rm --pull=always --platform linux/amd64 ghcr.io/astral-sh/uv:0.13.4-python3.11-trixie-slim dpkg-query -W ca-certificates
docker run --rm --pull=always --platform linux/amd64 python:3.11-slim-trixie dpkg-query -W ca-certificates
```

The commands are shown as the equivalent container reproduction, not as commands successfully run during this investigation. The following daemon-free inspection produced the observations below.

### Executed inspection

Resolved both tags with `skopeo inspect --no-creds --override-os linux --override-arch amd64`, then copied the images by their immutable index digests into a temporary directory. An empty registry-auth file and temporary Docker/cache directories isolated the inspection from existing state. The copy commands were:

```sh
skopeo copy --src-no-creds --authfile "$task_dir/config/auth.json" \
  --override-os linux --override-arch amd64 \
  docker://ghcr.io/astral-sh/uv@sha256:f62fcb3f2b942d30f848720710df6e4bfdd28fb155521f3d19e4bae7a8d9bdd7 \
  "dir:$task_dir/uv-image"
skopeo copy --src-no-creds --authfile "$task_dir/config/auth.json" \
  --override-os linux --override-arch amd64 \
  docker://docker.io/library/python@sha256:0a236bf8f0b6013383df04d1cd02cb0abe5fbfa1a7012141d7ff9202b7cb30a6 \
  "dir:$task_dir/python-image"
```

Here `task_dir` was `/tmp/uv-22789.8F3orU`. The inspection script at `$task_dir/inspect_image.py` verified each downloaded layer’s SHA-256 digest, processed layers in manifest order with whiteout handling, and read only an allowlist of package, OS, and public certificate files. It did not extract or execute the container filesystem. Selected files included `/var/lib/dpkg/status`, `/etc/ssl/certs/ca-certificates.crt`, `/etc/ca-certificates.conf`, and the G3 public root certificate. The final package database was placed in a separate temporary `dpkg/status` directory for each image.

Host `dpkg-query`, pointed at those extracted databases, reproduced the exact reported versions:

```console
$ dpkg-query --admindir="$task_dir/uv-image/dpkg" -W ca-certificates
ca-certificates  20250419
$ dpkg-query --admindir="$task_dir/python-image/dpkg" -W ca-certificates
ca-certificates  20250419+deb13u1
```

Both package records report `Status: install ok installed` and `Architecture: all`.

| Observation | uv 0.13.4 Python 3.11 Trixie slim | Current upstream Python 3.11 Trixie slim |
| --- | --- | --- |
| Image creation time, UTC | 2026-10-20 14:51:30 | 2026-10-21 01:09:58 |
| Installed `ca-certificates` | `20250419` | `20250419+deb13u1` |
| G3 public certificate file | Present | Absent |
| G3 enabled in `ca-certificates.conf` | Yes | No entry |
| G3 loaded from generated CA bundle | Yes | No |
| Number of roots loaded from bundle | 150 | 148 |

The selected `linux/amd64` manifest digests were:

- uv image: `sha256:87326c4d0a61d6ceb38156db87298c3cf31ee72a69e21d28d6b4baa0bd73a007`.
- Upstream Python image: `sha256:b95327ce1439e687237841b0210677aeadd62d55a060a5f6bcb9c15bf6251dd3`.

A fresh host Python `ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)` loaded each extracted bundle with `load_verify_locations(cafile=...)`; `get_ca_certs()` included G3 only for the uv image. This used the image’s bundle explicitly, rather than the host’s default store or certifi.

An additional offline OpenSSL check used the G3 root from the uv image against each bundle, disabling other trust sources:

```sh
openssl verify -no-CApath -no-CAstore \
  -CAfile "$task_dir/uv-image/selected/ca-certificates.crt" \
  "$task_dir/uv-image/selected/emsign-g3.crt"
openssl verify -no-CApath -no-CAstore \
  -CAfile "$task_dir/python-image/selected/ca-certificates.crt" \
  "$task_dir/uv-image/selected/emsign-g3.crt"
```

The uv bundle returned `OK` (exit 0). The upstream bundle returned `error 18 at 0 depth lookup: self-signed certificate` and `verification failed` (exit 2). The tested certificate’s subject was `C = IN, OU = emSign PKI, O = eMudhra Technologies Limited, CN = emSign ECC Root CA - G3`; its SHA-256 fingerprint was `86:A1:EC:BA:08:9C:4A:8D:3B:BE:27:34:C6:12:BA:34:1D:81:3E:04:3C:F9:E8:A8:62:CD:5C:57:A3:6B:BE:6B`.

These measurements establish the stale package and retained trust anchor. They do not test an actual compromised server chain, execute the image’s Python/curl/git, or establish impact across every published variant or architecture. The older Python base’s reported 2026-10-06 creation time was not independently checked. No additional report details are necessary to reproduce the concrete package/trust-store discrepancy.

### Existing test coverage

Searched `crates/uv/tests/` and `crates/uv-client/tests/it/` for Docker image references, `dpkg-query`, `ca-certificates`, and eMudhra/emSign. No test was found that checks these published Debian image package versions or the presence of this root in their system bundles.

Read the setup, request helpers, and assertions for the nearby tests in `crates/uv-client/tests/it/ssl_certs.rs`:

- `test_webpki_roots_trusts_pypi` clears certificate environment overrides and asserts an HTTPS request to PyPI succeeds with bundled roots.
- `test_system_certs_trusts_pypi` enables system certificates and asserts that request succeeds.
- `test_system_certs_with_ssl_cert_file_replaces_system_roots` supplies a generated self-signed CA and asserts a PyPI request fails with a fatal TLS error, proving replacement of the default trust source.

These tests are gated by `test-pypi`; they cover trust-source selection, not freshness of Docker image system certificates. The invalid-certificate-path tests in `crates/uv/tests/it/network.rs` likewise concern configuration warnings, not published images. No repository tests were added or run, and no build was required.

## Related

- astral-sh/uv#22767 — Update CA certificates in Alpine Docker images (open, draft; status rechecked). Closest parallel remediation for the same CA incident. Its certificate-package upgrade condition excludes Debian images, so it does not track the requested Trixie rebuild.
- astral-sh/uv#22760 — Bump webpki-root-certs to 1.0.10 (merged). The certificate fix shipped in uv 0.13.4 removes emSign ECC Root CA - G3 from uv’s bundled roots. It changes only Cargo.lock and does not update the Docker base image’s system certificate package.
- astral-sh/uv#22777 — Update rustls-webpki after the eMudhra incident (merged). Adds rejection of compromised intermediate public keys in uv’s Rust certificate verifier. This further addresses the same incident but does not patch the system trust store used by other programs in derived images.
- astral-sh/uv#22773 — Stop updating EOL Python 3.9 and 3.10 docker images (merged). Cites compromised-CA fixes as a reason to stop publishing variants whose upstream images no longer receive security updates. It concerns retired Python tags, whereas this report concerns supported Python 3.11/Trixie images.
- astral-sh/uv#22787 — Bump version to 0.13.5 (merged). The requested alternative version bump merged before this issue opened. The release API still lists 0.13.4 as latest and returns 404 for 0.13.5; the bump alone does not establish publication of refreshed Docker images.

## Supporting evidence

### Timing and repository mechanism

The issue opened on 2026-10-21 at 08:21:14 UTC and had no comments when the issue context was collected. The image creation timestamps independently measured above agree with the reporter’s uv and refreshed Python image timestamps.

In `.github/workflows/build-docker.yml`, the image matrix includes Debian, buildpack-deps, and Python Trixie bases. The generated Dockerfile uses `FROM` with the upstream image and copies `uv` and `uvx` into it; it does not upgrade system packages. The workflow targets `linux/amd64` and `linux/arm64`. This implementation explains the separation of trust stores; the reproduction conclusion comes from downloaded artifacts and certificate checks, not source inspection alone.

`docs/guides/integration/docker.md` distinguishes binary-only distroless images from derived images and documents copying uv into a chosen upstream Python image. `docs/concepts/authentication/certificates.md` describes bundled Mozilla roots as uv’s default, with system and custom trust stores selected separately.

The published advisory GHSA-hq85-5cpj-mr62 states that uv 0.13.4 removes emSign ECC Root CA - G3 from its vendored bundle and that system/custom stores require separate checks. The advisory was read again during reproduction: https://github.com/astral-sh/uv/security/advisories/GHSA-hq85-5cpj-mr62

### Release-status caveat

astral-sh/uv#22787 merged on 2026-10-21 at 02:55:36 UTC, before the issue opened. Although the checkout’s changelog includes 0.13.5, the release API rechecked during reproduction returns 404 for that tag and lists 0.13.4, published on 2026-10-20 at 15:24:59 UTC, as latest. Registry contents of any 0.13.5 tags were not inspected.

The changelog also describes a managed-Python update from astral-sh/uv#22786. Its metadata change was subsequently reverted by astral-sh/uv#22788 because of an interpreter crash; the checkout again references python-build-standalone 20261013. Neither change upgrades Debian’s system certificate package. The managed-Python changelog paragraph is not evidence of remediation for this Docker image.

## Search coverage and excluded candidates

The context-gathering search covered astral-sh/uv open and closed issues and pull requests using authenticated gh. It distinguished stale system certificates, Docker rebuild/publication, and additional Debian variants. Literal searches included GHSA-hq85-5cpj-mr62, eMudhra/emSign, ca-certificates, 20250419, Trixie, and 0.13.4; conceptual searches covered root certificates, vulnerable/outdated bases, security updates, rebuild/refresh/republish, and internal:releases. PR search returned no hits even for known matches, so the search was supplemented with the latest 1,000 PRs across all states, changelog references, and direct inspection of candidate bodies, comments, and diffs.

Excluded astral-sh/uv#19236 and its fix astral-sh/uv#19253 (an unrelated CRL advisory), astral-sh/uv#15262 (missing image tags), and astral-sh/uv#12462 (a new Chainguard variant). Also inspected astral-sh/uv#13727 (application dependency caching) and astral-sh/uv#12871 (requests/certifi bundle selection), whose maintainer discussions concern different behavior. The Alpine-only scope of astral-sh/uv#22767 rules it out as a duplicate. No canonical Debian rebuild discussion was found; this search is not an exhaustive guarantee.

## Draft response

Confirmed for the published linux/amd64 Python 3.11 Trixie slim image: uv 0.13.4 contains ca-certificates 20250419 and still includes emSign ECC Root CA - G3 in its system trust bundle. The current upstream Python image contains 20250419+deb13u1 and removes that root. This is separate from the bundled-root fix in astral-sh/uv#22760, and the Alpine update in astral-sh/uv#22767 does not cover Debian.

The 0.13.5 version bump has merged in astral-sh/uv#22787, but refreshed image publication is not confirmed. The affected image needs a refresh against a patched base, with other Debian variants and both architectures checked before treating the broader request as resolved. The documented alternative is to start from a freshly pulled, verified patched Python base and copy /uv and /uvx from the uv 0.13.4 distroless image.

## Recommended next steps

1. Verify the remaining supported Debian-derived variants and `linux/arm64`; the reported `linux/amd64` Python 3.11 slim variant is confirmed affected at the recorded digest.
2. Decide whether to republish affected 0.13.4 tags or publish refreshed images with 0.13.5. Verify both installed package versions and actual generated trust bundles before considering the issue resolved.
3. Coordinate with the Alpine remediation while tracking Debian verification separately. A patched upstream Python base plus copied uv binaries is the documented composition pattern; no replacement image was built during reproduction.

Only this issue-context README was updated outside the checkout. Temporary registry downloads and inspection files remain under `/tmp/uv-22789.8F3orU`. No repository files, existing Docker state, or GitHub state were changed, and no containers or builds were run.
