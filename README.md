# Powershell Install Fails on Powershell 5.1

Issue: astral-sh/uv#21602

Classification: bug, reproduction needs more information

## Summary

The report shows the documented Windows standalone-install command failing before installation on
Windows PowerShell 5.1.26100.8655:

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
```

The screenshot contains two errors from `Invoke-Expression`: an empty `Command` value followed by
an isolated `<#` whose multiline-comment terminator is missing. The uv 0.12.13 installer currently
served by that URL has a blank line followed by a `<# ... #>` comment-based help block at the same
position. This is consistent with the script being supplied to `Invoke-Expression` one line at a
time, but the required Windows PowerShell 5.1 response behavior could not be tested independently
on the available Linux runner.

The final mirror response currently has no `Content-Type` header. That is a useful diagnostic lead,
not a confirmed cause: PowerShell Core 6.0.0 and 7.6.5 both materialized the same live response as
one `System.String` in independent checks.

## Reproduction

Outcome: `needs_more_information`.

The available environment was Linux x86_64 with installed `uv 0.12.13` and PowerShell Core 7.6.5;
Windows PowerShell 5.1 was not available. The command does not use Python. The live vanity URL
redirected to
`https://releases.astral.sh/installers/uv/latest/uv-installer.ps1`, which served the uv 0.12.13
installer without a `Content-Type` header. A pinned
`https://astral.sh/uv/0.12.13/install.ps1` request also reached the mirror and had the same response
shape.

The download stage was checked without executing the remotely downloaded installer:

```powershell
$response = Invoke-RestMethod https://astral.sh/uv/install.ps1
$response.GetType().FullName
($response -split "`n").Count
```

PowerShell Core 7.6.5 returned `System.String` containing 677 lines. A temporary PowerShell Core
6.0.0 installation returned the same type and line count. These are compatibility variants, not a
substitute for Windows PowerShell 5.1, so they do not disprove the report.

A harmless temporary fixture reproduced the screenshot's parser errors when it was deliberately
enumerated by line:

```powershell
# harmless fixture matching the installer's leading structure

<#
.SYNOPSIS
Minimal reproduction
#>

'reached-script-body'
```

```powershell
Get-Content ./minimal.ps1 | Invoke-Expression
# Cannot bind argument to parameter 'Command' because it is an empty string.
# The terminator '#>' is missing from the multiline comment.

Get-Content -Raw ./minimal.ps1 | Invoke-Expression
# reached-script-body
```

This confirms the proposed line-enumeration mechanism, but it does not confirm that
`Invoke-RestMethod` produces that enumeration on a clean Windows PowerShell 5.1 host. To complete
the reproduction, run the following non-executing diagnostics on the affected host and on another
clean Windows PowerShell 5.1 host if possible:

```powershell
$response = Invoke-RestMethod https://astral.sh/uv/install.ps1
$response.GetType().FullName
@($response).Count
$PSVersionTable
(Invoke-WebRequest -UseBasicParsing https://astral.sh/uv/install.ps1).Headers
```

The Windows edition/build and whether a proxy, endpoint-security product, or content-filtering
gateway is present are also needed to distinguish PowerShell 5.1 behavior from host-specific HTTP
response rewriting. Testing the reported download-to-file workaround on the same host would show
whether only pipeline materialization is affected:

```powershell
Invoke-RestMethod https://astral.sh/uv/install.ps1 -OutFile install-uv.ps1
powershell -ExecutionPolicy Bypass -File .\install-uv.ps1
```

No existing integration test exercises the documented `Invoke-RestMethod | Invoke-Expression`
standalone-install path. `crates/uv/tests/it/self_update.rs` covers `uv self update` and mocked
release metadata/download selection; it downloads an installer to a file and is not coverage for
PowerShell's pipeline response handling.

## Draft response

Thanks for the report. The screenshot is consistent with the downloaded script being passed to
`Invoke-Expression` one line at a time: a harmless fixture with the same blank line and `<# ... #>`
structure produces the same two errors when enumerated by line. We have not yet independently
confirmed why the live response is enumerated on Windows PowerShell 5.1; PowerShell Core 6.0.0 and
7.6.5 both receive it as one string.

Could you share the output of the non-executing response-type, header, and `$PSVersionTable`
diagnostics above, plus the Windows edition/build and whether a proxy or endpoint-security product
is in the request path? Please also confirm whether downloading the script with `-OutFile` and
running the saved file works on the same host.

## Classification

Keep the bug classification provisionally. The repository documents the reported command as the
Windows installation method, and the screenshot establishes a real pre-install parse failure on the
reported host. Independent evidence confirms what produces those exact errors, but does not yet
establish whether the trigger is Windows PowerShell 5.1 itself, the headerless mirror response, or a
host/network-specific transformation.

This is not currently identified as a duplicate. The closest issues cover different failure modes
involving the same PowerShell installer path.

## Related

- astral-sh/uv#10428 (open issue), “Doc and install: avoid security issue on Windows”: the closest
  same-command report. It concerns endpoint security blocking fileless execution and records a
  download-to-file workaround, not the empty-command and isolated-comment parser errors.
- astral-sh/uv#5460 (open issue), “uv self update: failed to execute installer (status: exit code:
  1) on Windows”: an adjacent Windows PowerShell/cargo-dist installer compatibility tracker. Its
  confirmed failure is execution policy when self-update launches a saved script in Windows
  PowerShell 5.1; astral-sh/uv#21602 already uses `-ExecutionPolicy ByPass` and fails while piping
  the standalone installer into `Invoke-Expression`.
- astral-sh/uv#18725 (merged pull request), “Publish installers to `/installers/uv/latest` on the
  mirror”: delivery-path context for the current URL. It introduced publishing the latest
  PowerShell installer at the mirror endpoint reached by the vanity URL. The endpoint/header
  difference remains a diagnostic lead; this pull request neither reported nor fixed the observed
  parser failure.

## Supporting evidence

- The screenshot contains `ParameterArgumentValidationErrorEmptyStringNotAllowed` followed by
  `MissingTerminatorMultiLineComment`, with `<#` shown as the input to `Invoke-Expression`.
- The current uv 0.12.13 installer contains a blank line immediately before its `<# ... #>` help
  block.
- Supplying an equivalent harmless fixture as enumerated lines produces both reported errors;
  supplying it as one raw string succeeds.
- The current final mirror response has no `Content-Type` header, while the canonical GitHub release
  asset response is `application/octet-stream`. Neither correlation confirms the cause.
