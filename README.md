# Bump `quinn-proto` to 0.11.17

Issue: astral-sh/uv#21177

Classification: enhancement

## Summary

The issue requests a targeted update of the `quinn-proto` entry in `Cargo.lock` from 0.11.15 to
0.11.17. The cited upstream release addresses two high-severity remote-memory-exhaustion
advisories: GHSA-qfwj-vfxf-92j2, involving a bypass of the stream-reassembly chunk guard, and
GHSA-2hv7-gw8g-gpq5, involving zero-length DATAGRAM frames bypassing receive-buffer accounting.

The checkout does contain `quinn-proto` 0.11.15 in `Cargo.lock`. However, uv configures reqwest
with default features disabled and does not enable reqwest's `http3` feature. In reqwest 0.13.4,
quinn is an optional dependency selected by `http3`. A locked Cargo reverse-dependency check across
all workspace targets and all dependency edge kinds reports no package depending on
`quinn-proto`. The affected crate is therefore retained in the lockfile but is not selected in
uv's resolved workspace build graph. A uv maintainer has also confirmed in astral-sh/uv#21177 that
the project does not use HTTP/3 or QUIC.

The reporter clarified that the practical impact is policy enforcement rather than runtime QUIC
exposure: their JFrog/Artifactory scan flags the locked version as critically vulnerable and blocks
uv's use in their CI/CD environment. This scanner result and policy consequence are user-reported;
the attached screenshot provides supporting context but has not been independently reproduced.
A uv maintainer subsequently characterized JFrog's alert as a false positive because the affected
HTTP/3/QUIC code is not used, advised the reporter to contact their JFrog representative, and said
the dependency will be updated in due course. Maintainers do not consider the advisories security
issues in uv itself.

No existing issue or pull request in astral-sh/uv tracks this update. The closest related work is
astral-sh/uv#20025, a prior merged lockfile-only security bump for the same optional dependency, as
well as the upstream fix and its 0.11.x backport used to publish `quinn-proto` 0.11.17.

## Maintainer decision

Maintainers do not consider GHSA-qfwj-vfxf-92j2 or GHSA-2hv7-gw8g-gpq5 security issues in uv
because uv does not use the affected HTTP/3/QUIC functionality. They consider the JFrog alert a
false positive and recommend that affected users contact their JFrog representative. The dependency
will still be updated in due course; no specific release or completion date was given.

## Classification

This is an enhancement because the requested change improves dependency hygiene without correcting
an established uv runtime defect. Upstream confirms that 0.11.17 fixes the cited vulnerabilities,
and the lockfile currently records the affected 0.11.15 version. At the same time, repository and
Cargo metadata confirm that quinn is optional behind reqwest's disabled `http3` feature, so the
vulnerable code is not part of uv's selected build graph. Maintainers confirmed that uv does not use
HTTP/3 or QUIC, do not treat these advisories as uv security issues, and plan to update the
dependency in due course. The evidence supports a maintenance update, not a claim that current uv
binaries expose the upstream QUIC behavior. The reported JFrog/Artifactory CI/CD block gives the
lockfile-only update concrete user impact while leaving the enhancement classification unchanged.

## Related

- astral-sh/uv#20025 — Merged prior update from `quinn-proto` 0.11.14 to 0.11.15 for an earlier
  remote-memory-exhaustion advisory. Its description explicitly records the same situation: uv did
  not enable reqwest's HTTP/3 feature and was not affected at runtime, but the lockfile entry was
  updated anyway. The pull request changed only `Cargo.lock`, making it strong repository precedent
  rather than a tracker for the new 0.11.17 bump.
- quinn-rs/quinn#2789 — Merged upstream implementation titled “Assorted remote memory use fixes.”
  It rolls up the remote-memory-exhaustion fixes cited by the 0.11.17 release, but it is neither an
  existing uv tracker nor a downstream uv implementation.
- quinn-rs/quinn#2790 — Merged 0.11.x backport of quinn-rs/quinn#2789 plus the version bump. This is
  the direct upstream release-preparation change behind `quinn-proto` 0.11.17.

No open or closed issue, or open, closed, or merged pull request, was found that tracks the current
0.11.17 update; astral-sh/uv#20025 is historical precedent for the same kind of lockfile-only bump.

## Supporting evidence

- `Cargo.lock` records `quinn-proto` 0.11.15 via quinn 0.11.9 and reqwest 0.13.4.
- The workspace's reqwest feature list includes HTTP/2 but not HTTP/3; reqwest declares quinn as an
  optional dependency of its `http3` feature.
- `cargo tree --locked --invert quinn-proto --edges all --workspace --target all` reports no reverse
  dependency, confirming that the lockfile entry is not selected for any workspace target or edge
  kind.
- Repository members confirmed in astral-sh/uv#21177 that uv does not use HTTP/3 or QUIC. They do
  not consider the advisories security issues in uv, characterize JFrog's alert as a false positive,
  recommend contacting JFrog, and intend to update the dependency in due course.
- The reporter says JFrog/Artifactory flags the current uv release for critical vulnerabilities due
  to this lockfile entry, causing their company policy to block uv in CI/CD. Treat the scanner
  finding and policy effect as a user report, not as evidence that uv ships active QUIC code.
- astral-sh/uv#20025 independently confirms that maintainers previously accepted a lockfile-only
  `quinn-proto` security update despite uv not enabling HTTP/3; its entire diff was two additions
  and two deletions in `Cargo.lock`.
- The upstream `quinn-proto` 0.11.17 release says it fixes the cited advisories and identifies
  quinn-rs/quinn#2789 and quinn-rs/quinn#2790 as the implementation and 0.11.x release backport.
- Literal searches covered `quinn-proto`, `0.11.17`, both GHSA identifiers, and the exact memory and
  DATAGRAM symptoms. Conceptual searches covered quinn/QUIC vulnerabilities, memory exhaustion,
  eviction loops, and dependency-security advisories across every issue and pull-request state.
  Only astral-sh/uv#21177 matched the substantive report.
- astral-sh/uv#19630 surfaced in the bare `0.11.17` search solely because its reporter used uv
  0.11.17. Its corrupt tool-receipt failure and maintainer discussion are unrelated to quinn,
  HTTP/3, dependency updates, or either advisory.
