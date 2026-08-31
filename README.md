# For Windows app, a comprehensive description would be nice

Issue: astral-sh/uv#21190

Classification: bug

## Summary

The reporter asks for a more informative presentation of uv in Windows 10 after installation. The
screenshot shows the Windows Installed Apps entry with a generic icon, the name `uv`, version
`0.12.5`, an installation date, and uninstall controls, but no visible publisher or other product
information. The body reports version `0.12.15`, although the screenshot shows `0.12.5`, which was
the current release when the issue was filed. The body version therefore appears to be a typo.

The closest previous report, astral-sh/uv#18967, requested Product Name and Company Name in the
Windows executable's File Properties as well as code signing. It was closed as a duplicate of the
open signing issue astral-sh/uv#10336. The draft implementation in astral-sh/uv#18280 signs release
binaries but does not add the Installed Apps description, publisher, icon, or executable version
resources requested here.

The reporter has now clarified that Unsloth's setup installed uv with
`winget install --id=astral-sh.uv -e`. Inspection of `unslothai/unsloth`'s `install.ps1` at commit
`8059be6bca04cf3fcab2f8d3bdc54155a8560635`, the latest revision before this issue was filed, shows
that the setup first runs WinGet upgrade/install for package `astral-sh.uv` when WinGet is
available. If WinGet is unavailable or does not yield a usable uv, it falls back to downloading a
pinned uv release archive, copying the executables into a user bin directory, and adding that
directory to `PATH`.

The reporter's explicit command confirms that the observed entry comes from WinGet's portable
package registration. The WinGet locale manifest already contains publisher, description, project,
support, documentation, and license metadata, but the 0.12.5 installer manifest did not include an
`AppsAndFeaturesEntries` block to populate the corresponding registration fields.

microsoft/winget-pkgs#426812 is an open downstream pull request for the 0.12.7 manifest. It adds an
`AppsAndFeaturesEntries` entry with `DisplayName: uv`, `Publisher: Astral Software Inc.`, and
`DisplayVersion: 0.12.7`. The pull request is approved and automated validation has completed, but
it has not merged. It directly addresses the missing publisher shown in the screenshot; it does not
add a description, support URL, or application icon, so it should not yet be treated as resolving
every possible meaning of the reporter's request for a comprehensive description.

## Reproduction and remaining information

A representative path is a Windows 10 system with WinGet available and no acceptable uv already on
`PATH`, followed by `winget install --id=astral-sh.uv -e`. The resulting uv entry should then be
inspected under Settings > Apps. Unsloth's Windows setup invokes the same package installation path.

The reporter still needs to identify the specific desired fields, such as publisher, description,
support URL, or application icon. Once microsoft/winget-pkgs#426812 merges and the updated manifest
is published, reinstalling or upgrading uv 0.12.7 through WinGet can verify whether the publisher
now appears. Description, support-link, and icon expectations require separate clarification because
the pull request does not add those fields.

## Classification

This is now best classified as a bug. The confirmed WinGet installation path and the diff in
microsoft/winget-pkgs#426812 establish that the portable installer manifest omitted the
`AppsAndFeaturesEntries` fields needed to register uv's known publisher in Windows Settings. The
missing publisher is therefore a concrete downstream packaging defect rather than only a request
for aesthetic improvement. Broader requests for a description, support URL, or icon remain
enhancements unless Windows is expected to expose those fields. It is not a duplicate because the
closest canonical discussion, astral-sh/uv#10336, tracks executable signing and verified publisher
identity rather than WinGet's Installed Apps registration.

## Related

- astral-sh/uv#18967 — Closed issue and the closest prior request. It explicitly asks for Product
  Name and Company Name in Windows File Properties, but it also bundles code signing and concerns
  executable metadata rather than the Installed Apps registration shown here. It was closed as a
  duplicate of astral-sh/uv#10336.
- astral-sh/uv#10336 — Open canonical issue for signing published Windows executables. A verified
  publisher may be one aspect of the reporter's request, but this issue does not track an Installed
  Apps description, application icon, or registration metadata.
- astral-sh/uv#18280 — Open draft pull request implementing Windows and macOS release-binary
  signing. It provides the current implementation status for astral-sh/uv#10336 but does not add
  the richer Installed Apps or executable version metadata requested here.
- astral-sh/uv#11456 — Open discussion about making WinGet the preferred Windows installation
  method. Maintainer and WinGet comments explain that WinGet's portable-package handling creates
  the registry records used by Control Panel and Add or Remove Programs, but the issue itself is
  about installation guidance rather than richer app metadata.
- microsoft/winget-pkgs#426812 — Open downstream pull request adding `DisplayName`, `Publisher`, and
  `DisplayVersion` to the uv 0.12.7 manifest's `AppsAndFeaturesEntries`. It is approved and validated
  but not merged, and its scope does not include a description, support URL, or icon.

## Search scope and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal terms
included the issue title, `comprehensive description`, `Apps & Features`, `Installed apps`, `Add and
Remove Programs`, `Product Name`, `Company Name`, `File Properties`, `DisplayIcon`, and
`AppsAndFeaturesEntries`. Conceptual searches covered Windows installer and application metadata,
executable resources and version information, generic/application icons, publisher information,
WinGet portable packages and registry entries, MSI metadata, uninstall entries, and code signing.
Fix-oriented searches included closed metadata and signing reports and merged or open release-binary
pull requests.

The strongest chain inspected was astral-sh/uv#18967 to astral-sh/uv#10336 and
astral-sh/uv#18280. astral-sh/uv#20815 was also inspected but ruled out as a related item: it concerns
a WinGet release being blocked by a missing manifest property and was treated as a downstream
packaging problem, not the presentation of an installed entry. The current WinGet 0.12.5 manifests
were checked and already contain rich locale metadata, so they do not establish that the same
metadata is expected to appear in Windows Installed Apps. The reporter's later identification of
Unsloth as the installing application was checked against Unsloth's installer source as it existed
when the issue was filed. The reporter subsequently confirmed that the WinGet command was used.
microsoft/winget-pkgs#426812 was inspected and confirms that the proposed downstream change is to
add the three missing Apps and Features registration fields for 0.12.7; it remains open. No merged
pull request was found that had previously added and then regressed this behavior.
