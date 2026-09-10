# Powershell Install Fails on Powershell 5.1

Issue: astral-sh/uv#21602

Classification: bug

## Summary

The documented standalone-install command fails before installation under Windows PowerShell
5.1.26100.8655:

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
```

The screenshot shows `Invoke-Expression` first rejecting an empty `Command` value and then parsing
`<#` by itself, producing `MissingTerminatorMultiLineComment`. The currently published installer
does begin a multiline comment-based help block with `<#`, so the immediate incompatibility is
supported by the report and published script. The suggestion that response handling or headers
cause Windows PowerShell 5.1 to enumerate the download line-by-line is plausible, but has not been
confirmed independently.

No existing issue or pull request tracks this exact empty-command/multiline-comment failure. The
closest items cover other failure modes involving the same PowerShell installer path.

## Draft response

Thanks for the report. The screenshot shows Windows PowerShell 5.1 invoking `iex` with an empty
value and then with `<#` by itself. The current installer uses `<# ... #>` for its help block, so
the documented one-liner is not executing the downloaded script as one unit in this environment.
That is distinct from the execution-policy problem in astral-sh/uv#5460 and the security-software
behavior in astral-sh/uv#10428.

As a workaround, could you download and run the script as a file and confirm whether it completes
on the same host?

```powershell
irm https://astral.sh/uv/install.ps1 -OutFile install-uv.ps1
powershell -ExecutionPolicy Bypass -File .\install-uv.ps1
```

The output is consistent with line-by-line pipeline handling, but we have not yet confirmed why
the response is materialized that way on this PowerShell version.

## Classification

This is a bug. The repository presents the reported command as the Windows installation method,
but the captured output establishes that it cannot parse the published installer under the stated
Windows PowerShell version. The failure occurs before download or installation logic runs.

It is not a duplicate: none of the searched open or closed issues, or open, closed, and merged pull
requests, reported the same empty `Command` followed by an isolated `<#` and
`MissingTerminatorMultiLineComment`. Similar issues instead involve execution policy, endpoint
security, networking, or PATH refresh.

## Related

- astral-sh/uv#10428 (open issue), “Doc and install: avoid security issue on Windows”: the closest
  same-command report. It concerns the documented `irm ... | iex` pipeline and records a
  download-to-file workaround, but the trigger is endpoint security blocking fileless execution,
  not PowerShell 5.1 splitting the script.
- astral-sh/uv#5460 (open issue), “uv self update: failed to execute installer (status: exit code:
  1) on Windows”: an adjacent Windows PowerShell/cargo-dist installer compatibility tracker. Its
  confirmed failure is execution policy when self-update launches a saved script in Windows
  PowerShell 5.1; astral-sh/uv#21602 already uses `-ExecutionPolicy ByPass` and fails earlier while
  piping the standalone installer into `Invoke-Expression`.
- astral-sh/uv#18725 (merged pull request), “Publish installers to `/installers/uv/latest` on the
  mirror”: delivery-path context for the exact current URL. It introduced publishing the latest
  PowerShell installer at the mirror endpoint now reached by the vanity URL. The endpoint/header
  difference is only a diagnostic lead; this pull request neither reported nor fixed the
  PowerShell 5.1 parsing failure.

## Supporting evidence

- The screenshot contains `ParameterArgumentValidationErrorEmptyStringNotAllowed` followed by
  `MissingTerminatorMultiLineComment`, with `<#` shown as the input to `iex`.
- The current published `install.ps1` contains a blank line immediately before a `<# ... #>` help
  block. That structure explains the two immediate errors if pipeline input is enumerated by line.
- astral-sh/uv#10428 demonstrates that saving `install.ps1` locally before executing it is already
  a known alternative for a different `irm ... | iex` failure.
- The vanity installer URL currently redirects to the mirror location introduced by
  astral-sh/uv#18725. Older release responses included `Content-Type: application/x-powershell`,
  while the current mirror response does not expose that header. This correlation is worth testing
  on Windows PowerShell 5.1, but it does not yet establish the root cause.

## Search coverage and ruled-out candidates

Literal searches covered `PowerShell 5.1` and `Powershell 5.1`, `install.ps1`, `irm`, `iex`,
`Invoke-RestMethod`, `Invoke-Expression`, and the exact screenshot errors. Conceptual searches
covered legacy/Classic PowerShell, Windows installer compatibility, string-array and line-by-line
pipeline behavior, multiline comments, comment-based help, cargo-dist, execution policy, and
security/fileless execution. Version- and fix-oriented searches covered uv 0.12.13, cargo-dist
0.32.0, release hosting, and pull requests in every state.

The strongest candidates, their comments, and referenced discussions were inspected.
astral-sh/uv#2286 uses the exact installer command but reports the older execution-policy error,
which `-ExecutionPolicy ByPass` avoids. astral-sh/uv#14583 and astral-sh/uv#3116 both reach the
installation stage and concern PATH refresh afterward. Windows installer reports involving TLS,
proxy handling, antivirus, architecture, and artifact downloads also fail at materially different
stages.
