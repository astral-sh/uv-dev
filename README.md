# eMudhra remove additional C1 and C3 certs

Issue: astral-sh/uv#23269

Classification: duplicate

## Summary

The reporter requests updating Cargo.lock to webpki-root-certs 1.0.11 or later to remove
“emSign Root CA - C1” and “emSign ECC Root CA - C3” from uv's bundled trusted roots.
They distinguish this from the earlier G3 removal and offer to contribute if the pending
dependency update needs additional work.

astral-sh/uv#2658 already tracks the required 1.0.12 dependency bump, while Cargo.lock remains at 1.0.10. astral-sh/uv#22760 removed G3 only; astral-sh/uv#22777 and astral-sh/uv#22789 address intermediate-key rejection and Docker system certificates, respectively.

The open dashboard's specific webpki-root-certs update is listed under “Pending Status Checks”;
it is not a merged fix, and no corresponding update pull request was found.

## Draft response

The lockfile still pins webpki-root-certs 1.0.10. Upstream 1.0.11 removes C1 and C3, and the 1.0.12 update already listed under “Pending Status Checks” in astral-sh/uv#2658 includes those removals. Let's track the dependency bump there. The earlier astral-sh/uv#22760 update removed G3 only.

## Classification

Open astral-sh/uv#2658 explicitly tracks the same webpki-root-certs lockfile update, targeting 1.0.12, which satisfies the requested minimum and includes the C1/C3 removals. The dependency-bump request can be centralized there, so duplicate takes precedence. The October 20 G3 fix in astral-sh/uv#22760 predates the October 23 C1/C3 removal upstream; this is an additional distrust update, not a regression of that fix.

The source confirms a stale bundled trust list rather than a request for a new TLS capability:
uv loads the roots from the locked crate by default. The duplicate classification reflects the
existing tracker for the exact dependency update. It does not imply that the removal has shipped.
There is no evidence here establishing a successful attack or failure of the separate
intermediate-key rejection mitigation.

## Related

- astral-sh/uv#2658 — Dependency Dashboard (open issue). Explicitly lists the webpki-root-certs 1.0.12 update under Pending Status Checks. That version includes the C1/C3 removals introduced in 1.0.11, so this open tracker already covers the requested lockfile bump; no corresponding update PR was found.
- astral-sh/uv#22760 — Bump webpki-root-certs to 1.0.10 (merged pull request). Merged October 20, 2026, changing only Cargo.lock to remove emSign ECC Root CA - G3. C1/C3 were removed upstream in 1.0.11 on October 23, so this earlier fix does not cover the additional roots and has not regressed.
- astral-sh/uv#22777 — Update rustls-webpki after the eMudhra incident (merged pull request). Updates rustls-webpki to 0.103.16 to reject chains containing compromised intermediate public keys. The checkout already includes that version; this mitigation is distinct from removing C1/C3 from the bundled root list.
- astral-sh/uv#22789 — Rebuild 0.13.4 Docker images to pick up patched ca-certificates (eMudhra CA compromise) (closed issue). Same CA incident, but concerns Docker base-image system ca-certificates used by tools such as curl, Git and Python, rather than uv's bundled webpki-root-certs. A maintainer reports it fixed in uv 0.13.5; that does not establish C1/C3 removal from uv's bundle.

## Supporting evidence

- Cargo.lock:8472 pins webpki-root-certs to 1.0.10. Cargo.toml:366 allows version 1,
  so 1.0.12 fits the declared dependency requirement.
- crates/uv-client/src/tls.rs:209 loads webpki_root_certs::TLS_SERVER_ROOT_CERTS.
  crates/uv-client/src/base_client.rs:650–655 selects these roots when neither custom
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
- astral-sh/uv#2658 explicitly lists “Update Rust crate webpki-root-certs to v1.0.12”
  under “Pending Status Checks” and has no discussion comments.
- Cargo.lock:4298 already pins rustls-webpki to 0.103.16, the intermediate-key mitigation
  from astral-sh/uv#22777. CHANGELOG.md records that mitigation in uv 0.13.5 and the
  earlier bundled-root update in uv 0.13.4.
- The closing maintainer comment on astral-sh/uv#22789 says uv 0.13.5 fixes the Docker
  system-certificate issue. It addresses a different certificate source.

## Search scope and excluded candidates

Searched astral-sh/uv open and closed issues and open, closed and merged PRs with authenticated gh. Literal searches covered eMudhra, emSign, C1/C3, 2083709, webpki-root-certs, and versions 1.0.11/1.0.12. Conceptual searches covered root certificates, trust stores, certificate removal, distrust and compromised certificates; fix-oriented searches covered webpki and bundled-root updates. Inspected the reporter's leads, candidate discussions, relevant release notes and upstream removal PR rustls/webpki-roots#136. Because search omitted known PRs, also inspected 400 recent PR records spanning October 19–November 16 and the Renovate branch's PR history. No dedicated C1/C3 update PR was found. Ruled out astral-sh/uv#22767 as an Alpine system-CA update and astral-sh/uv#23081 as a macOS CI trust-store change; neither updates uv's bundled roots.

The inspected repository candidates did not identify another canonical discussion for C1/C3
removal beyond the exact dependency update listed in astral-sh/uv#2658. Older Renovate PRs on
the same branch target 1.0.9 or earlier and do not satisfy this request.

## Maintainer next step

Advance the existing webpki-root-certs 1.0.12 dashboard entry or prepare the same focused lockfile
update with `cargo update -p webpki-root-certs --precise 1.0.12`, followed by normal dependency-update
validation. Keep bundled roots, operating-system trust stores, and managed Python certificate
bundles distinct when describing the scope of the update.
