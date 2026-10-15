# macOS release binaries: use a stable code-signing Identifier for uv and uvx

Issue: astral-sh/uv#22627

Classification: bug

## Summary

The reporter finds that official macOS uv and uvx executables retain different hash-suffixed code-signing identifiers in releases 0.12.12, 0.12.24, and 0.13.1, while their Team ID remains 2DC432GLL2. They report that upgrades and reinstalls invalidate identity-based Little Snitch rules and designated-requirement allowlists, causing repeated approval prompts. They request stable, distinct identifiers for uv and uvx, including the binaries packaged in wheels.

astral-sh/uv#14870 establishes the signing history; astral-sh/uv#21754 and astral-sh/uv#21526 identify the shared archive/wheel signing pipeline. No existing issue or PR was found tracking stable identifiers.

## Draft response

The release signer currently calls rcodesign without --binary-identifier, consistent with the changing identifiers you reported. astral-sh/uv#14870 established Developer ID signing in 0.12.12, but the identifier-stability problem remains separate. Official release archives and PyPI wheels use the same signing pipeline. The next step is to configure distinct, stable identifiers for uv and uvx in the shared signing action and verify their identifiers and designated requirements across releases in both distribution formats.

## Classification

The report describes release identity changes that invalidate existing rules for already signed executables. Both the 0.12.12 signer and the currently pinned signer omit an explicit binary identifier, consistent with the supplied codesign output. The original signing request was completed, but identifier stability was not established; no previously fixed stability bug or existing duplicate was found. The reported macOS prompts were not independently reproduced.

This is a follow-up correctness problem in signed releases, rather than a request to introduce signing. The original issue concerned absent Developer ID signatures. Its completion does not establish that identities remain compatible across upgrades. The shared-action refactor and later signature-page-size change postdate the first affected version reported here.

## Related

- astral-sh/uv#14870 — Sign published executables for macOS (closed issue). The original request covers Developer ID signing of uv and uvx, including organizational allowlists and repeated approvals after upgrades. Maintainers closed it when signing shipped in 0.12.12 and confirmed that official archives and PyPI wheels contain signed binaries. The new report concerns changing identifiers in those signed binaries, so it is a distinct remaining defect.
- astral-sh/uv#21754 — Use shared release signing actions (merged pull request). Moved signing, notarization, assembly, and verification into astral-sh/github-actions, following astral-sh/github-actions#3. The currently pinned shared signer invokes rcodesign without --binary-identifier, identifying the implementation that needs investigation. This refactor did not establish stable identifiers.
- astral-sh/uv#21526 — Sign release binaries from the wheel inventory (merged pull request). Signs the executable inventory extracted from wheels and verifies the expected executables in both wheels and GitHub archives. This connects the report's two distribution formats to the same signing pipeline; it does not address identifier stability.

## Supporting evidence

### Reported observations

The report gives the following results from inspecting the contents of the aarch64 macOS release archive with `codesign -dv` on 2026-10-15:

| Release | uv Identifier | uvx Identifier |
| --- | --- | --- |
| 0.12.12 | uv-924a20a6cc8daa66 | uvx-52a8c5d340e3db83 |
| 0.12.24 | uv-7d60a5a1146de3cd | uvx-462788ef10328e23 |
| 0.13.1 | uv-12607afb3fd970b7 | uvx-1704e7899e715f4e |

The supplied designated requirement includes the release-specific identifier and the stable Team ID. The reporter attributes the identifiers to the linker's existing ad-hoc signature. Archive metadata and firewall prompts were not independently reproduced, and the report does not provide equivalent wheel measurements or an MDM/PPPC reproduction.

### Repository and maintainer evidence

- In astral-sh/uv#14870, maintainers confirmed on 2026-09-09 that signing shipped in 0.12.12 and that the curl installer, GitHub releases, and PyPI wheels contain signed binaries. They clarified that Homebrew builds its own binaries. The release was published on 2026-09-09.
- The [0.12.12 signing script](https://github.com/astral-sh/uv/blob/0.12.12/scripts/sign-release-macos.py#L150) invokes `rcodesign sign` without `--binary-identifier`.
- The checkout's `.github/workflows/sign-release-binaries.yml:160` pins `astral-sh/github-actions/sign-macos` to commit `a30f3520f325da7fed035847afec51629d0ef4fa`. The [pinned signing script](https://github.com/astral-sh/github-actions/blob/a30f3520f325da7fed035847afec51629d0ef4fa/sign-macos/scripts/sign-release-macos.py#L178) likewise supplies no binary identifier and uses `--config-file /dev/null`. This confirms the missing explicit setting; it is consistent with, but does not independently verify, the reporter's explanation of inherited identifiers.
- astral-sh/uv#21526 verifies the wheel and archive executable inventories. astral-sh/uv#21754 adopted the shared signing and assembly implementation from astral-sh/github-actions#3. That shared implementation inserts signed executable bytes into both distribution formats.
- astral-sh/uv#22246, merged on 2026-10-06 and following astral-sh/github-actions#8, changes ARM64 signature pages to 16 KiB. Its scope is signature-verification performance, not stable program identity.

## Search scope and excluded candidates

Used authenticated gh searches across open/closed issues and open/closed/merged PRs. Literal searches covered uv-12607afb3fd970b7, sh.astral.uv, signing Identifier, designated requirement, binary-identifier, Little Snitch, and program identity changed. Conceptual searches covered code identity, permissions after updates, firewall rules, allowlists, Santa, MDM/PPPC, and keychain approvals. Implementation/fix searches covered codesign, rcodesign, notarization, macOS signing, wheels, and release labels. PR search omitted known matching changes, so it was supplemented with signing-workflow commit history, all open PR titles, recent closed PR listings, and direct inspection of referenced PRs and comments. Compared the 0.12.12 and current signing source. Ruled out astral-sh/uv#16726 (managed Python SIGKILL), astral-sh/uv#8069 (self-update mirrors), astral-sh/uv#22219 (ad-hoc dylib signing), and astral-sh/uv#22246 (signature page size). Open signing prototypes astral-sh/uv#20660 and astral-sh/uv#20641 also omit explicit identifiers and do not track this request. No existing stable-identifier fix or duplicate was found.

The strongest superficially similar issue is astral-sh/uv#16726: it concerns uv-managed Python interpreters being killed, with maintainers requesting SDK and MDM details. It does not concern uv or uvx identities changing after an upgrade. astral-sh/uv#8069 concerns downloading self-updates through a mirror behind a firewall. astral-sh/uv#22219 concerns ad-hoc signatures for edited Mach-O dylibs. None supplies a canonical discussion for this report.

The open prototypes astral-sh/uv#20660 and astral-sh/uv#20641 concern earlier wheel-signing/assembly approaches. Their diffs were inspected: neither sets an explicit identifier. The closed, unmerged astral-sh/uv#21552 was superseded by astral-sh/uv#21754. No matching stable-identifier fix was found in these chains.

GitHub PR search did not return known signing changes, so negative search results alone were not used to infer that no related work exists. Workflow history and direct PR reads supplied the implementation history above. A transient search rate limit was followed by successful additional searches.

## Recommended next step

Define stable, distinct identities for uv and uvx in the shared macOS signing action, with the exact identifier names decided by maintainers. The reporter's `sh.astral.uv` and `sh.astral.uvx` are suggestions, not an established naming contract.

Validate the Identifier, TeamIdentifier, and designated requirement in the final archives and wheels for both supported macOS architectures, then compare separately built versions to verify that an identity-based rule continues to match. A macOS check is still needed to confirm the reported firewall behavior. No release timeline or completed fix is established.

Only this temporary handoff README was updated. No checkout files or GitHub state were changed.
