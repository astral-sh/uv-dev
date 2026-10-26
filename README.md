# Please update 'quinn-proto',  'zerovec-derive', 'zerovec'

Issue: astral-sh/uv#22936

Classification: enhancement

## Summary

The reporter says Aikido flagged three Rust dependencies as vulnerable and recommends
quinn-proto 0.11.19, zerovec-derive 0.11.5, and zerovec 0.11.8. The issue was opened on
2026-10-26. The original report supplies no advisory identifiers, affected uv version,
scan target, triggering input, or observed command failure.

astral-sh/uv#21177 and astral-sh/uv#20025 explain earlier quinn-proto alerts and uv's disabled HTTP/3 support. Closed, unmerged astral-sh/uv#19853 proposed older zerovec updates. No existing item was found covering all three requested versions.

The checked-out lockfile is below all three requested targets. Repository member EliteTK
has assessed the reported dependencies and states that the vulnerabilities have no impact
on current uv usage: quinn-proto is not compiled into uv, and the zerovec crates parse
ICU data compiled into uv rather than untrusted data. EliteTK intends to look into version
bumps anyway; no completed update, linked implementation PR, or release date is supplied.

## Current maintainer assessment

Source: EliteTK's comment on astral-sh/uv#22936, posted 2026-10-26 at 20:41:28 UTC
(comment ID 6426830952). The supplied full discussion identifies EliteTK as a repository
MEMBER.

- **quinn-proto:** EliteTK reports checking that it is present in Cargo.lock but is not
  compiled as part of uv's build. Its lockfile inclusion comes from a weak reference in
  reqwest's `rustls` feature.
- **zerovec-derive and zerovec:** EliteTK reports that their relevant use is through
  `icu_normalizer`, parsing ICU data compiled into uv. On that basis, the affected parsing
  does not receive untrusted data.
- **Impact and disposition:** EliteTK concludes that the reported vulnerabilities have no
  impact on current uv usage and will look into bumping the versions anyway. This is an
  intention to investigate maintenance updates, not confirmation of a fix or release.

These are attributed maintainer findings from the issue discussion. The existing local
source checks below establish locked versions and feature configuration; this follow-up
did not independently rebuild uv or audit all ICU parsing paths. The assessment concerns
the reported vulnerabilities in uv's current usage, not the safety of these crates for
other consumers or configurations.

## Classification

Classification remains enhancement: the request is to update dependencies, and the
maintainer's applicability assessment identifies no security impact on current uv usage.
It adds a direct assessment of both zerovec crates to the earlier evidence about disabled
QUIC functionality. There is no established uv correctness defect or regression requiring
bug classification, and the related items concern earlier updates rather than this
three-crate request.

The absence of advisory identifiers in the original report remains a documentation gap,
but the current disposition is supported by the maintainer's build and data-flow findings,
rather than solely by the absence of a reproducer. New evidence that affected code handles
untrusted data in a supported uv configuration would warrant revisiting the assessment.

The earlier quinn-proto issue requested 0.11.17 and the merged PR updated to 0.11.15;
the current checkout already locks 0.11.18. Neither establishes a returned bug.
The closed Rust dependency refresh proposed older zerovec versions and was never merged.
These items provide related history without a canonical discussion that covers the
present three-crate request.

## Related

- astral-sh/uv#21177 — Bump `quinn-proto` to 0.11.17 (issue, closed). Earlier scanner-driven quinn-proto upgrade request for two QUIC advisories. Maintainers explained that uv does not use HTTP/3 or QUIC and those findings did not affect uv. This informs the quinn-proto assessment, but does not identify the new Aikido alerts or cover either zerovec crate.
- astral-sh/uv#20025 — Bump `quinn-proto` to 0.11.15 (pull request, merged). Merged on 2026-06-27, updating quinn-proto from 0.11.14 to 0.11.15 for GHSA-4w2j-m93h-cj5j. Its description explicitly says uv was unaffected because reqwest's http3 feature is disabled. This is historical maintenance precedent, not a fix for the requested 0.11.19 update.
- astral-sh/uv#19853 — Update Rust dependencies (pull request, closed). An earlier, unmerged dependency refresh proposed zerovec 0.11.5 to 0.11.6 and zerovec-derive 0.11.2 to 0.11.3. It provides update history for the other two crates, but neither reaches the requested versions nor identifies the reported vulnerabilities.

## Supporting evidence

### Checked-out source

| Dependency | Locked version | Requested version | Source |
| --- | --- | --- | --- |
| quinn-proto | 0.11.18 | 0.11.19 | `Cargo.lock:3572` |
| zerovec-derive | 0.11.2 | 0.11.5 | `Cargo.lock:9048` |
| zerovec | 0.11.5 | 0.11.8 | `Cargo.lock:9037` |

At `Cargo.toml:261`, reqwest has default features disabled and an explicit feature
list containing http2 but not http3. No Rust source or Cargo manifest in the checkout
mentions enabling http3. This supports the maintainers' prior explanation about QUIC;
presence in Cargo.lock alone does not prove the crate is compiled into uv.

Cargo.lock records quinn-proto as a dependency of quinn. It records zerovec beneath
ICU, potential_utf, and tinystr, and zerovec-derive beneath zerovec. This identifies
their dependency context without establishing vulnerability reachability.

### Maintainer discussion and history

In astral-sh/uv#21177, zanieb states that uv does not use HTTP/3 or QUIC.
woodruffw explains that the reported JFrog findings are false positives for uv's usage
and that the dependency will be updated through normal maintenance. Those comments
address GHSA-qfwj-vfxf-92j2 and GHSA-2hv7-gw8g-gpq5, not unidentified future alerts.

astral-sh/uv#20025 independently records the disabled reqwest http3 feature and a
maintenance bump for GHSA-4w2j-m93h-cj5j. It merged on 2026-06-27, before this report.
astral-sh/uv#19853 was opened on 2026-06-15 and closed without merging; its diff shows
only the older zerovec targets described above.

For general maintenance context, astral-sh/uv#20733 contains a maintainer explanation
that dependencies are reviewed manually and automatically and updates use cooldowns.
That discussion concerns dependency review policy, not these particular alerts.

### Advisory cross-check

The public GitHub advisory API returned GHSA-xrv3-jmcp-374j for zerovec and
GHSA-74r5-g7vc-j2v2 for zerovec-derive. Both describe incorrect use of
`#[repr(packed)]` in older releases. The returned ranges end before 0.10.4 for
zerovec and before 0.10.3 for zerovec-derive, with separate fixes for the 0.9 series.
Neither returned range includes the locked 0.11 versions.

The returned public quinn-proto advisories have fixes at 0.11.15 or earlier.
These public results do not identify the Aikido findings requiring the requested
versions and do not establish that Aikido has no additional advisory.

## Search scope and excluded candidates

Used authenticated gh searches across open/closed issues and open/closed/merged PRs, separately querying quinn-proto, zerovec-derive, zerovec, and the requested versions. Broadened to Aikido, vulnerability, security scanner, RustSec, dependency updates, HTTP/3, ICU, zero-copy, memory safety, and unsoundness; also searched the public zerovec advisory identifiers. PR search omitted a known matching PR, so supplemented it by examining titles and bodies of the 3,000 most recent PRs, spanning 2026-05-05 through 2026-10-26. Inspected candidate discussions, maintainer comments, linked PRs, and relevant diffs. Ruled out astral-sh/uv#22475: its reqwest update does not change the three requested crates. Aikido reports astral-sh/uv#20541 and astral-sh/uv#20542 concern different crates and advisories. Checked Cargo.lock, reqwest features, and public GitHub advisories; found no canonical discussion covering all three requested updates.

Literal package/version searches were kept separate from broader scanner and dependency
maintenance searches. Searches for ICU, zero-copy, memory safety, unsoundness, and public
advisory identifiers explored possible mechanisms without treating them as the cause
of the reported alerts. Searches had no state restriction; direct PR enumeration also
included open, closed, and merged results.

The open Dependency Dashboard, astral-sh/uv#2658, led to astral-sh/uv#22475.
The latter's body, comments, and diff were inspected: it updates reqwest to 0.13.5
without changing the three requested crates. It is not a matching fix.

Other excluded candidates were astral-sh/uv#20541 (open crate Aikido findings),
astral-sh/uv#20542 (quick-xml Aikido findings), and astral-sh/uv#19236 with its linked
merged update astral-sh/uv#19253 (rustls-webpki CRL handling). They concern different
advisories and do not establish the applicability of these alerts.
astral-sh/uv#20065 updates container digests, and astral-sh/uv#20640 changes resolver
behavior; their generic dependency titles do not describe the requested crate updates.

## Next step

Follow up on the optional dependency bumps discussed by EliteTK and record a matching PR
or released version when one is available. The supplied discussion does not establish
that any of the requested updates has landed or that a security-priority release is needed.
If later evidence challenges the applicability assessment, obtain the exact advisory,
affected uv build/configuration, and path by which untrusted data reaches the affected code.

The initial `triage.json` records the assessment before this maintainer comment; its draft
reply and unresolved applicability wording are superseded by the current context above.
The classification and related items remain unchanged. No public response was drafted or
posted for this follow-up, and only this README was updated.
