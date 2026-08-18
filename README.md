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

The screenshot is consistent with the registry entry created for a WinGet portable installation,
but the reporter did not identify the installation method. The current WinGet 0.12.5 locale
manifest already contains publisher, description, project, support, documentation, and license
metadata, while its installer manifest registers uv as a portable application. That makes the
installation method and the exact desired fields necessary to determine whether a change belongs in
the downstream WinGet registration or in uv's Windows executable resources.

## Draft response

Thanks. The screenshot shows a Windows Installed Apps entry with only uv's name and version, plus a
generic icon. The current WinGet manifest for 0.12.5 already contains publisher, description,
project, support, documentation, and license metadata. astral-sh/uv#18967 previously requested
metadata on `uv.exe` and was redirected to the code-signing discussion in astral-sh/uv#10336, but
those issues do not establish which layer is responsible for the entry shown here.

Could you confirm how uv was installed, especially whether you used
`winget install --id=astral-sh.uv -e`, and which fields you want Windows to show—for example the
publisher, a description or support link, or an application icon? Also, the screenshot appears to
show 0.12.5 while the report says 0.12.15; please confirm the installed version. With those details
we can determine whether this belongs in WinGet's portable-package registration or uv's executable
resources.

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
metadata is expected to appear in Windows Installed Apps. No merged pull request was found that had
previously added and then regressed this behavior.
