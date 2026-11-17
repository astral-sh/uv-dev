# Move to an rsa >= 0.10 (RUSTSEC-2023-0071) once reqsign supports it

Issue: astral-sh/uv#23279

Classification: enhancement

## Summary

The reporter requests adopting a reqsign release that supports rsa >=0.10 to clear RUSTSEC-2023-0071 / CVE-2023-49092 from uv's Rust dependency graph. They report scanner findings for uv 0.13.9 and main and identify cloud endpoint authentication as the relevant subsystem. No existing uv discussion was found that tracks this RSA upgrade.

The checkout confirms rsa 0.9.10 through reqsign 0.20.7, including reqsign-core 3.3.2, reqsign-google 3.2.0, and reqsign-azure-storage 3.3.0. RustSec lists >=0.10.0 as patched. The upstream requirement upgrade, apache/reqsign#941, remains open with failing checks. Its diff changes the shared rsa requirement from 0.9.2 to 0.10.0 without implementation changes.

## Draft response

Your dependency report matches the current lockfile: reqsign still brings in rsa 0.9.10, and RustSec lists >=0.10.0 as patched. apache/reqsign#941 is still open with failing checks. This issue can track adopting a compatible reqsign release; the next step is an upstream release, followed by a uv dependency update that removes rsa 0.9.x from the lockfile. The dependency finding alone does not establish whether the timing attack is reachable through uv's signing operations.

## Classification

This requests a dependency upgrade once upstream supports it, so enhancement fits. The affected dependency's presence and advisory are confirmed, but the inspected evidence does not establish a uv correctness defect or a reachable timing attack through uv. This classification does not treat the advisory as harmless. No equivalent uv tracker was found, and the historical JWT advisory and earlier reqsign updates concern different problems. There is no evidence of a previously fixed RSA problem returning.

## Related

- apache/reqsign#941 — open pull request, “build(deps): update rsa requirement from 0.9.2 to 0.10.0”. This is the exact upstream requirement upgrade needed for the requested dependency migration. Created November 16, 2026; it remains unmerged with failing build, check, and unit-test statuses. The only substantive comment is from this issue's reporter; the specific API incompatibility explanation has not been independently confirmed by a maintainer or by inspecting failure logs. This upstream change does not itself perform the uv dependency update.
- astral-sh/uv#18287 — closed issue, “jsonwebtoken Rust crate (9.3.1) in Cargo.lock is affected by CVE-2026-25537”. Closest historical parallel: scanners detected a transitive cryptographic dependency through reqsign-google, and a major dependency update required upstream support. A maintainer explained that the JWT vulnerability affected decoding while uv's dependency used encoding. That conclusion applies to the JWT advisory and cannot establish the impact of this RSA advisory. It is not the same request.
- astral-sh/uv#22925 — merged pull request, “Update Rust crate reqsign to v0.20.7”. Merged October 26, 2026; its release notes list the reqsign component versions now present in the checkout. It predates the reporter's claimed November RSA release and does not upgrade rsa to 0.10. It is historical dependency-update context, not an RSA fix or duplicate tracker.

## Supporting evidence

- `Cargo.toml:255` enables reqsign's AWS, Azure, Google, and default-context features. `Cargo.lock:3848` locks reqsign 0.20.7; entries beginning at lines 3899, 3931, and 3966 list rsa under the Azure, core, and Google components. `Cargo.lock:4156` locks rsa 0.9.10.
- `crates/uv-auth/src/providers.rs:62` onward configures reqsign signers for UV_S3_ENDPOINT_URL, UV_GCS_ENDPOINT_URL, and UV_AZURE_ENDPOINT_URL. This supports the reported subsystem connection. It does not show that all three providers use RSA on every request or establish exploitability, key recovery, or the reporter's claim about every scanned binary.
- The RustSec advisory at https://github.com/RustSec/advisory-db/blob/main/crates/rsa/RUSTSEC-2023-0071.md describes private-key recovery through timing side channels and lists >=0.10.0 as patched, citing constant-time mitigations and migration to crypto-bigint.
- apache/reqsign#941 changes the shared requirement to rsa 0.10.0. Its status checks confirm failures, but their precise cause was not independently established. No compatible release was established by the inspected evidence. A GitHub release lookup for the RSA v0.10.0 tag returned 404, so the reporter's exact November 8 publication date was not independently verified; the patched-version statement is independently supported by RustSec.
- The timeline for astral-sh/uv#18287 points to merged astral-sh/uv#17935 and closed alternative astral-sh/uv#18672. The latter repeats the JWT upgrade and contains a maintainer comment referring back to the decoding-versus-encoding explanation. These are historical JWT discussions, not an RSA fix.

## Search scope and excluded candidates

The report was separated into: (1) rsa 0.9.10 and the exact advisory/CVE identifiers; (2) adopting rsa >=0.10 through reqsign and its named component crates; (3) cloud endpoint signing and the claimed upstream compatibility blocker; and (4) scanner findings in distributed binaries. Searches of astral-sh/uv used the authenticated gh CLI across open and closed issues and open, closed, and merged pull requests.

Literal searches covered RUSTSEC-2023-0071, CVE-2023-49092, GHSA-c38w-74pg-36hr, rsa, reqsign, and reqsign-core. Conceptual searches covered Marvin, timing side channels, vulnerable dependencies, security dependencies, scanners/binaries, endpoint vulnerabilities, signing security, and the security label. Searches without specific package versions checked the underlying advisory and dependency-maintenance concerns. Fix-oriented searches explicitly included closed/merged rsa 0.10 and reqsign pull requests. Indexed PR searches returned no matching results, so referenced historical PRs were fetched directly and their discussion inspected.

astral-sh/uv#22906 was inspected and ruled out as the same request: its reqsign-core 3.3.2 update addressed broken PPC32 code in the mea dependency, replaced by asyncband; its maintainer comment links astral-sh/uv#22925. astral-sh/uv#22273 was inspected and ruled out: it concerns silent Google identity fallback after a credential configuration parsing failure, not RSA versions or timing attacks. astral-sh/uv#2658 contains a reqsign dependency inventory but no specific RSA upgrade tracker. The JWT issue's referenced update PRs were followed to distinguish their earlier fix from the current request.

## Next step

Track the upstream requirement migration and a compatible published reqsign release, then evaluate a focused uv dependency update and verify that the resolved lockfile no longer contains rsa 0.9.x. Assess any claim about reachable timing attacks separately from the dependency inventory. No uv release date or exploitability conclusion is established here.
