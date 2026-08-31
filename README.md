# For Windows app, a comprehensive description would be nice

Issue: astral-sh/uv#21190

Classification: enhancement

## Summary

The reporter asks for a more informative presentation of uv in Windows 10 after installation. The
screenshot shows the Windows Installed Apps entry with a generic icon, the name `uv`, version
`0.12.5`, an installation date, and uninstall controls, but no visible publisher or other product
information. The body reports version `0.12.15`, although the screenshot shows `0.12.5` and `0.12.5`
is the current release, so the version should be confirmed.

The closest previous report, astral-sh/uv#18967, requested Product Name and Company Name in the
Windows executable's File Properties as well as code signing. It was closed as a duplicate of the
open signing issue astral-sh/uv#10336. The draft implementation in astral-sh/uv#18280 signs release
binaries but does not add the Installed Apps description, publisher, icon, or executable version
resources requested here.

The reporter has now clarified that uv was installed automatically by Unsloth's setup. Inspection
of `unslothai/unsloth`'s `install.ps1` at commit
`8059be6bca04cf3fcab2f8d3bdc54155a8560635`, the latest revision before this issue was filed, shows
that the setup first runs WinGet upgrade/install for package `astral-sh.uv` when WinGet is
available. If WinGet is unavailable or does not yield a usable uv, it falls back to downloading a
pinned uv release archive, copying the executables into a user bin directory, and adding that
directory to `PATH`.

The direct-download fallback does not create an Add or Remove Programs registration, whereas the
screenshot shows such a registered entry. Together, the reporter's clarification and the Unsloth
installer source make the WinGet portable-package path the strongest explanation for the observed
UI, although an Unsloth installation log or `winget list --id=astral-sh.uv -e` result would confirm
which branch actually ran. The current WinGet 0.12.5 locale manifest already contains publisher,
description, project, support, documentation, and license metadata, while its installer manifest
registers uv as a portable application. The remaining investigation is therefore whether WinGet
can propagate richer manifest fields into the Windows entry and, separately, whether uv should add
an application icon or version resources to its executables.

## Reproduction and remaining information

A representative path is a Windows 10 system with WinGet available and no acceptable uv already on
`PATH`, followed by running Unsloth's Windows setup. At the revision inspected, the setup attempts
`winget upgrade --id=astral-sh.uv -e --source winget` and then
`winget install --id=astral-sh.uv -e --source winget` if needed. The resulting uv entry should then
be inspected under Settings > Apps.

The reporter still needs to identify the specific desired fields, such as publisher, description,
support URL, or application icon. An Unsloth installation log or the result of
`winget list --id=astral-sh.uv -e` would remove the remaining uncertainty about whether WinGet
completed the installation rather than the direct-download fallback.

## Classification

This is an enhancement. The screenshot establishes that the Windows entry is sparse, but the name
and displayed version are not shown to be incorrect. The request is therefore for richer product
metadata and presentation. It is not a duplicate because the closest canonical discussion,
astral-sh/uv#10336, tracks executable signing and verified publisher identity rather than the
Installed Apps fields shown here. If the reporter identifies a field that Windows or WinGet should
already propagate but does not, the classification can be revisited as a bug with a concrete
installation path.

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
when the issue was filed. That source confirms WinGet as the preferred uv installation path and a
direct archive installation as the fallback. No merged pull request was found that had previously
added and then regressed this behavior.
