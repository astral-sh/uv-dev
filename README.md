# Naming of uv not found as a cmdlet, function, scriptfile, executable program

Issue: astral-sh/uv#22488

Classification: duplicate

## Summary

The reporter installs uv with WinGet on Windows 11, then receives PowerShell's command-not-found error when invoking `uv`. They ask whether antivirus software is responsible. Their motivation is to run GameForge-Harness; the request for another Unity harness is background context, not a separate uv capability request.

astral-sh/uv#14583 is the closest existing support discussion; astral-sh/uv#3116 corroborates the shell-refresh advice. The older PATH bug and antivirus tracker provide context but do not establish a current defect.

The screenshot provides more precise versions than the issue text: uv 0.13.0 and Python 3.13.14. The prose's Python 3.14 description conflicts with the screenshot and version field. Python compatibility is not implicated by an error raised before uv starts.

## Draft response

Your screenshot shows WinGet installing uv 0.13.0 successfully and saying “Pfadumgebungsvariable geändert; Starten Sie Ihre Shell neu, um den neuen Wert zu verwenden” — restart your shell to use the updated PATH. Please fully close PowerShell/Windows Terminal, reopen it, and run `uv --version`.

This repeats the post-installation command-discovery question in astral-sh/uv#14583. The error means PowerShell cannot locate uv; it does not by itself indicate antivirus interference. If it still fails after restarting, please share the output of `winget list --id astral-sh.uv -e` and `Get-Command uv -All`, and whether your antivirus reports quarantining uv.

## Classification

The post-installation PowerShell command-discovery support question is already covered by astral-sh/uv#14583, so duplicate takes precedence over question. The new WinGet trigger includes an explicit instruction to restart the shell before the same CommandNotFoundException occurs. No failure after following that instruction, antivirus detection, uv runtime defect, or regression of the historically fixed standalone-installer bug is demonstrated.

The duplicate target is the existing post-installation support discussion, not the historical installer bug or the antivirus tracker. WinGet is a different installation route, but the report presents the same immediate command-discovery question and the same unperformed shell-refresh step. The exact PATH contents and the result after restarting remain unknown; successful recovery has not been confirmed.

## Related

- astral-sh/uv#14583 (closed; can't install uv through powershell): Closest match: invoking uv immediately after installation produces the same PowerShell CommandNotFoundException. The reply explains PATH visibility and recommends restarting PowerShell. That report used the standalone installer; this report uses WinGet, whose screenshot explicitly requests a shell restart.
- astral-sh/uv#3116 (closed; uv powershell windows only works if you are running powershell as admin): Reports successful installation followed by the same unrecognized-uv error. Multiple commenters resolved command discovery by restarting PowerShell, logging out, or rebooting; a maintainer confirms administrator access should not be required. The original standalone-installer/admin trigger differs from this WinGet session.
- astral-sh/uv#10014 (closed; `uv` command on Windows: Inconsistent PATH set by installer): Historical PATH-propagation bug with the identical PowerShell error, but it persisted in newly opened shells after standalone installation. Its upstream fix, axodotdev/cargo-dist#1658, removed the system-restart requirement while retaining a shell restart. The current WinGet screenshot shows no shell restart, so it does not establish recurrence of that bug.
- astral-sh/uv#20792 (open; Windows antivirus/EDR issues): Canonical context for the reporter's antivirus question, including quarantined/deleted executables and access-denied errors. This report shows neither a detection nor quarantine evidence, so antivirus interference is unconfirmed and this is not the duplicate target.

## Supporting evidence

### Report and screenshot

The issue has no comments at the time of review. Its screenshot shows this sequence:

1. `python --version` reports Python 3.13.14.
2. `winget install --id=astral-sh.uv -e` selects uv 0.13.0 and downloads the Windows x86-64 release archive.
3. WinGet reports successful hash verification, extraction, and installation, with command aliases for `uvx`, `uv`, and `uvw`.
4. WinGet prints: “Pfadumgebungsvariable geändert; Starten Sie Ihre Shell neu, um den neuen Wert zu verwenden.” This explicitly instructs the user to restart the shell for the updated PATH.
5. The next visible command is `uv` in the same PowerShell window. It fails with `ObjectNotFound` and `CommandNotFoundException`.

These observations support trying the requested shell restart first. They do not prove that the executable was quarantined, that installation failed, or that a Python version change is needed. No failure in a freshly restarted terminal is reported.

### Existing discussions and historical fix

In astral-sh/uv#14583, the July 13, 2025 reply identifies PATH visibility as the likely explanation for the same immediate PowerShell error and recommends restarting PowerShell after a successful installation. The issue was closed on July 16, 2025; it does not include a reporter-confirmed diagnosis.

In astral-sh/uv#3116, the original reporter said on November 26, 2024 that logging out or rebooting made uv available because PATH needed reloading. Later commenters confirmed that reopening PowerShell worked. A maintainer said on February 12, 2025 that administrator access should not be necessary and asked about PATH and restarting.

astral-sh/uv#10014 concerns a more specific historical defect: PATH visibility differed between newly launched shells after the standalone PowerShell installer. A maintainer directed the discussion to axodotdev/cargo-dist#1614. Following that discussion and its timeline identifies axodotdev/cargo-dist#1658, “Rework Add-Path to avoid expanding and request an environment refresh,” merged January 7, 2025. Its description explicitly says a shell restart is still required, while a full system restart is no longer required. The uv issue closed January 15, 2025. This historical fix does not demonstrate a WinGet regression in uv 0.13.0.

astral-sh/uv#20792 tracks Windows antivirus/EDR interference, including deleted executables and access-denied failures. A maintainer's September 9, 2026 comment notes that binaries have been signed since uv 0.12.12, without claiming this eliminates false positives. The report contains no antivirus alert or protection-history evidence.

The checkout's `docs/getting-started/installation.md`, “WinGet” section, documents the exact installation command used in the screenshot. `docs/reference/installer.md` describes the standalone installer's PATH changes; those standalone-specific paths should not be substituted for WinGet's installation location.

## Search coverage and excluded candidates

Searched astral-sh/uv open and closed issues using cmdlet, CommandNotFoundException, not recognized, winget with not found/PowerShell/PATH, and installer-independent command not found/install terms. Conceptual searches covered shell restarts, PATH visibility, Windows installation, and antivirus/EDR separately; the GameForge-Harness identifier returned only this report. Searched open, closed, and merged PRs for WinGet, Windows installer, PowerShell/PATH, cargo-dist, antivirus, the exact exception, and version 0.13.0; no matching uv PR was returned. Read candidate comments and followed astral-sh/uv#10014 through axodotdev/cargo-dist#1614 to merged axodotdev/cargo-dist#1658. Ruled out astral-sh/uv#12211 (WinGet installation fails with Unicode errors), astral-sh/uv#14693 (misleading update-shell output after uv runs), and astral-sh/uv#2287 (pip/Python installation PATH setup).

The initial search separated the observable successful-install/command-not-found sequence from the reporter's antivirus hypothesis. The labels and discussions supplied additional vocabulary: `area:windows`, `internal:releases`, `external`, `tracking`, shell refresh, PATH propagation, and quarantine.

The especially plausible alternatives were checked against their actual failure stages:

- astral-sh/uv#12211 fails during WinGet installation with a Unicode/code-page error. Here WinGet reports success and PowerShell subsequently cannot locate the command.
- astral-sh/uv#14693 concerns contradictory PATH messages from `uv python install` and `uv tool update-shell`, including stale VS Code environments. Here uv never starts and no VS Code session is reported.
- astral-sh/uv#2287 involves `pip install uv` and Python's scripts-directory setup. Here WinGet downloads the standalone executable.
- axodotdev/cargo-dist#833, initially suggested in astral-sh/uv#3116, concerns execution policy blocking the standalone installer. No such policy error appears in this WinGet report.

## Next step

Ask the reporter to fully restart the terminal application and run `uv --version`. If command discovery still fails, the two targeted diagnostic commands in the draft establish whether WinGet still lists the package and whether PowerShell resolves any uv executable. Request antivirus quarantine evidence only as a diagnostic observation, without presuming antivirus is responsible.
