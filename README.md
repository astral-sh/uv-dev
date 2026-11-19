# eMudhra remove additional C1 and C3 certs

Issue: astral-sh/uv#23269

Classification: duplicate

Status: closed as completed; fixed on main, with no published release containing the fix as of November 19, 2026.

## Summary

The reporter requests updating Cargo.lock to webpki-root-certs 1.0.11 or later to remove
“emSign Root CA - C1” and “emSign ECC Root CA - C3” from uv's bundled trusted roots.
They distinguish this from the earlier G3 removal and offer to contribute if the pending
dependency update needs additional work.

astral-sh/uv#23302 merged on November 19, 2026, updating Cargo.lock from webpki-root-certs
1.0.10 to 1.0.12 and closing this issue. The pin on main is now 1.0.12, which includes the
C1/C3 removals introduced in 1.0.11. The latest published release at verification, uv 0.13.9,
still pins 1.0.10.

On November 19, 2026, a uv maintainer said the C1/C3 removal should go through a normal
dependency bump. Their assessment was that the intermediates associated with these roots
were not known to have weak keys, unlike G3B/G3C, and they asked for any evidence to the
contrary. The upstream removal concerns the two root certificates; it does not establish
that their intermediates have the same weak-key problem as G3B/G3C.

## Classification

At triage, astral-sh/uv#2658 already tracked the same webpki-root-certs lockfile update to
1.0.12. That existing entry is the basis for the duplicate classification. The subsequent
astral-sh/uv#23302 is the fix. The October 20 G3 fix in astral-sh/uv#22760 predates the
October 23 C1/C3 removal upstream, so the later removals are separate from that fix.

uv loads the roots from the locked crate by default. The old pin kept C1/C3 in that bundle;
the merged update removes them. There is no evidence here establishing a successful attack
or failure of the separate intermediate-key rejection mitigation.

## Related

- astral-sh/uv#23302 — Bump webpki-root-certs (merged pull request). Updated only Cargo.lock from 1.0.10 to 1.0.12 and closed this issue.
- astral-sh/uv#2658 — Dependency Dashboard (open issue). At triage, it listed the webpki-root-certs 1.0.12 update under Pending Status Checks. That entry covered the requested lockfile bump and is no longer listed following the merge.
- astral-sh/uv#22760 — Bump webpki-root-certs to 1.0.10 (merged pull request). Merged October 20, 2026, changing only Cargo.lock to remove emSign ECC Root CA - G3. C1/C3 were removed upstream in 1.0.11 on October 23, so this earlier fix does not cover the additional roots and has not regressed.
- astral-sh/uv#22777 — Update rustls-webpki after the eMudhra incident (merged pull request). Updates rustls-webpki to 0.103.16 to reject chains containing compromised intermediate public keys. The checkout already includes that version; this mitigation is distinct from removing C1/C3 from the bundled root list.
- astral-sh/uv#22789 — Rebuild 0.13.4 Docker images to pick up patched ca-certificates (eMudhra CA compromise) (closed issue). Same CA incident, but concerns Docker base-image system ca-certificates used by tools such as curl, Git and Python, rather than uv's bundled webpki-root-certs. The reporter confirmed their Docker issue was fixed in uv 0.13.5.

## Supporting evidence

- The lockfile diff in astral-sh/uv#23302 changes the webpki-root-certs version and checksum.
  Cargo.toml:366 allows version 1. The published 0.13.9 tag still pins 1.0.10, while main
  pins 1.0.12.
- crates/uv-client/src/tls.rs:209 loads webpki_root_certs::TLS_SERVER_ROOT_CERTS.
  crates/uv-client/src/base_client.rs:655–660 selects these roots when neither custom
  certificates nor system certificates are selected.
  docs/concepts/authentication/certificates.md:20 documents bundled Mozilla roots as the default.
- Upstream release 1.0.10, published October 19, removes G3 only:
  https://github.com/rustls/webpki-roots/releases/tag/v/1.0.10
- Upstream release 1.0.11, published October 23, explicitly removes C1 and C3 and cites
  Mozilla bug 2083709:
  https://github.com/rustls/webpki-roots/releases/tag/v/1.0.11
  Its merged removal change, rustls/webpki-roots#136, modifies both the webpki-roots
  and webpki-root-certs certificate lists.
- Upstream release 1.0.12, published November 10, follows 1.0.11 and removes additional
  unrelated roots:
  https://github.com/rustls/webpki-roots/releases/tag/v/1.0.12
- Cargo.lock:4298 already pins rustls-webpki to 0.103.16, the intermediate-key mitigation
  from astral-sh/uv#22777. CHANGELOG.md records that mitigation in uv 0.13.5 and the
  earlier bundled-root update in uv 0.13.4.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed and merged PRs with authenticated gh. Literal searches covered eMudhra, emSign, C1/C3, 2083709, webpki-root-certs, and versions 1.0.11/1.0.12. Conceptual searches covered root certificates, trust stores, certificate removal, distrust and compromised certificates; fix-oriented searches covered webpki and bundled-root updates. Inspected the reporter's leads, candidate discussions, relevant release notes and upstream removal PR rustls/webpki-roots#136. Because search omitted known PRs, also inspected 400 recent PR records spanning October 19–November 16 and the Renovate branch's PR history. No dedicated C1/C3 update PR existed at triage; astral-sh/uv#23302 was opened and merged later. Ruled out astral-sh/uv#22767 as an Alpine system-CA update and astral-sh/uv#23081 as a macOS CI trust-store change; neither updates uv's bundled roots.

Older Renovate PRs on the same branch target 1.0.9 or earlier and do not satisfy this request.

## Maintainer next step

The lockfile update is complete on main. If release availability matters, check the first published
tag that includes astral-sh/uv#23302. The change applies to uv's bundled roots; system trust stores
and managed Python certificate bundles are separate.
