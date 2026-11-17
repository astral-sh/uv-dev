# Move to an rsa >= 0.10 (RUSTSEC-2023-0071) once reqsign supports it

Issue: astral-sh/uv#23279

Classification: enhancement

## Summary

The reporter requests adopting a reqsign release that supports rsa >=0.10 to clear RUSTSEC-2023-0071 / CVE-2023-49092 from uv's Rust dependency graph. They report scanner findings for uv 0.13.9 and main and identify cloud endpoint authentication as the relevant subsystem. No existing uv discussion was found that tracks this RSA upgrade.

The checkout confirms rsa 0.9.10 through reqsign 0.20.7, including reqsign-core 3.3.2, reqsign-google 3.2.0, and reqsign-azure-storage 3.3.0. RustSec lists >=0.10.0 as patched. The upstream requirement upgrade, apache/reqsign#941, remains open with failing checks. Its diff changes the shared rsa requirement from 0.9.2 to 0.10.0 without implementation changes.

Repository member woodruffw assesses exploitation through uv as unlikely, citing the distinction between decryption oracles and uv’s RSA signing paths, apparent signature blinding, and limited opportunity for an attacker to repeatedly trigger the operation. This is a qualified maintainer assessment, not an independently verified finding that uv is unaffected.

The maintainer’s disposition is to receive the upgrade transitively when available, with no separate uv-side action considered actionable for this indirect dependency. woodruffw recommends reporting the scanner finding to a paid scanner vendor as a false positive; this recommendation reflects their exploitability assessment rather than new independent verification.

## Maintainer disposition

woodruffw considers this indirect-dependency report not otherwise actionable on uv’s side and expects uv to receive the version bump transitively when available. They recommend that users of a paid security scanner report the finding to its vendor as a false positive. This is maintainer guidance on handling the report, not evidence that the dependency has already been upgraded or an independently confirmed non-applicability determination. No release date, issue-closure decision, or commitment to a special uv-side fix is stated.

Source: woodruffw, repository member, November 17, 2026; comment 6957093275 on astral-sh/uv#23279. https://github.com/astral-sh/uv/issues/23279#issuecomment-6957093275

## Classification

This requests a dependency upgrade once upstream supports it, so enhancement fits. The affected dependency's presence and advisory are confirmed, but the inspected evidence does not establish a uv correctness defect or a reachable timing attack through uv. This classification does not treat the advisory as harmless. No equivalent uv tracker was found, and the historical JWT advisory and earlier reqsign updates concern different problems. There is no evidence of a previously fixed RSA problem returning.

## Maintainer assessment of exploitability

In astral-sh/uv#23279, repository member woodruffw states that they do not think the issue is exploitable through uv. They explain that Marvin-style attacks generally target decryption with PKCS#1 v1.5 padding as an oracle, whereas uv’s relevant paths sign with RSA keys and, to their understanding, use blinded signatures. They also identify a separate constraint: an attacker needs to repeatedly drive the oracular signing or decryption operation, for which they believe uv offers limited opportunity.

The comment is tentative, including the observation about blinding. The signing implementation and attacker-controlled repetition have not been independently verified in this investigation. Record this as the maintainer’s preliminary risk assessment rather than a confirmed non-applicability determination. It does not change the confirmed dependency inventory or the request to adopt a compatible reqsign release.

Source: woodruffw, repository member, November 17, 2026; comment 6957056349 on astral-sh/uv#23279. https://github.com/astral-sh/uv/issues/23279#issuecomment-6957056349

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

Follow the maintainer’s disposition: await the transitive upgrade through a compatible reqsign release rather than treating this report as requiring a separate uv-side fix. When that update becomes available, verify that the resolved lockfile no longer contains rsa 0.9.x. For users of a paid scanner, the maintainer recommends reporting the finding to the vendor as a false positive, citing the signing-path assessment above. If further exploitability investigation is needed, check the maintainer’s stated assumptions about the RSA signing and blinding paths and whether an attacker can repeatedly trigger observable private-key operations. Keep that assessment separate from the dependency inventory. No uv release date or definitive exploitability determination is established here; the maintainer’s current assessment is that exploitation through uv is unlikely.
