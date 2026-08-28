# Uv 0.12.7 and 0.12.6 detected as a virus

Issue: astral-sh/uv#21336

Classification: duplicate

## Summary

The reporter cannot install uv 0.12.6 or 0.12.7 on Windows 11 because Netskope detects and removes it as a virus. The report does not identify the installation method, downloaded artifact, architecture, Netskope detection name, or vendor support case.

astral-sh/uv#20792 is the canonical same-problem tracker: it explicitly names Netskope, applies to Windows AV/EDR interference, and includes uv being quarantined or deleted. astral-sh/uv#10428 contains an earlier Netskope report affecting recurring uv releases. Windows release signing, a relevant repository-side mitigation, is tracked in astral-sh/uv#10336 and the open implementation astral-sh/uv#18280.

## Draft response

Thanks for the report. This is covered by astral-sh/uv#20792, which specifically tracks Netskope and cases where Windows AV/EDR software quarantines or deletes uv. Please contact Netskope or your organization’s security administrator first and obtain a vendor support ticket; then add the ticket ID, the exact Netskope detection name/event, the affected artifact and architecture, and your installation method to astral-sh/uv#20792. Windows release signing is tracked separately in astral-sh/uv#10336 and astral-sh/uv#18280, but it is not yet enabled for published releases.

## Classification

This is a duplicate of astral-sh/uv#20792. That open tracking issue already covers the same underlying behavior, the same Windows environment category, the same AV/EDR vendor, and the same quarantine/deletion outcome. The affected uv versions are additional observations, but the report does not establish a distinct regression or failure mode that needs a separate discussion.

The repository evidence establishes that Netskope is interfering with installation by removing or blocking uv. It does not establish that the uv artifacts contain malware, that the detections are caused by unsigned binaries, or that signing will resolve this particular Netskope detection. Those remain unconfirmed without the exact detection details and vendor analysis.

## Related

- astral-sh/uv#20792 — **Windows antivirus/EDR issues** (open issue). This is the canonical match: it explicitly lists Netskope, covers all Windows versions, and includes uv being quarantined or deleted. Maintainers ask affected users to contact their AV/EDR vendor, obtain a support ticket, and share the ticket ID.
- astral-sh/uv#10428 — **Doc and install: avoid security issue on Windows** (open issue). A later comment reports that Netskope detects and blocks uv as malware on every release, particularly the x64 build. The main issue is adjacent rather than canonical because it primarily discusses security software blocking the PowerShell installation command.
- astral-sh/uv#10336 — **Sign published executables for Windows** (open issue). This tracks signing Windows release executables specifically to reduce antivirus reports. It is a related mitigation discussion, not confirmation of the detection’s root cause.
- astral-sh/uv#18280 — **Add code signing of release binaries via `cargo-code-sign`** (open pull request). This implements release-binary signing for Windows and macOS, but remains open and states that production release credentials were not configured.

## Search evidence

Literal searches covered `Netskope`, uv 0.12.6 and 0.12.7, virus detection and removal, and installation blocking. Conceptual searches covered antivirus/AV, EDR, quarantine, malware and false-positive terminology, unsigned executables, SmartScreen, and Windows code signing. Fix-oriented searches included open and closed issues and open, closed, and merged pull requests, with inspection of comments and referenced issue chains.

astral-sh/uv#20567 and related Windows runtime file-lock reports were inspected but ruled out as less direct. They concern AV/EDR products transiently locking trampoline or PE-resource files during uv operations, whereas astral-sh/uv#21336 reports Netskope removing or blocking a uv release during installation.
