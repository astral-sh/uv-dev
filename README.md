# Uv 0.12.7 and 0.12.6 removed by netskope

Issue: astral-sh/uv#21336

Classification: duplicate

## Summary

The reporter cannot install uv 0.12.6 or 0.12.7 on Windows 11 because Netskope detects and removes it as a virus. A uv maintainer confirms that the detection is a false positive. The reporter states that the 0.12.5 installer works, providing a potentially useful version boundary. The report does not identify the installation method, downloaded artifact, architecture, Netskope detection name, or vendor support case.

astral-sh/uv#20792 is the canonical same-problem tracker: it explicitly names Netskope, applies to Windows AV/EDR interference, and includes uv being quarantined or deleted. A maintainer directs affected users there and asks them to work through their commercial relationship with the vendor, since uv cannot prevent vendor false positives. astral-sh/uv#10428 contains an earlier Netskope report affecting recurring uv releases. Published binaries are signed starting with uv 0.12.12, completing the request tracked by astral-sh/uv#10336. Maintainers do not expect signing to resolve AV/EDR detections immediately, but expect it to help over the longer term because vendors can also flag signed binaries falsely.

## Classification

This is a duplicate of astral-sh/uv#20792. That open tracking issue already covers the same underlying behavior, the same Windows environment category, the same AV/EDR vendor, and the same quarantine/deletion outcome. The reported transition from a working 0.12.5 installer to blocked 0.12.6 and 0.12.7 installers is useful for investigation, but it does not establish a different failure mode that needs a separate discussion.

The repository evidence establishes that Netskope is interfering with installation by removing or blocking uv, and a uv maintainer confirms that this is a false positive rather than malware in the uv artifacts. Maintainers direct remediation to Netskope through the affected organization’s commercial relationship and state that uv cannot prevent vendors from producing false positives. A commenter attributes this detection to the affected executables being unsigned and the publisher therefore being unavailable for exclusion, but provides no supporting diagnostic or vendor evidence. Binaries are now signed as of uv 0.12.12, while the affected releases predate that change. Maintainers explicitly caution that signed binaries can still receive false-positive detections and do not expect signing to resolve AV/EDR issues immediately. The discussion therefore does not establish that lack of signing caused this particular detection.

## Related

- astral-sh/uv#20792 — **Windows antivirus/EDR issues** (open issue). This is the canonical match: it explicitly lists Netskope, covers all Windows versions, and includes uv being quarantined or deleted. Maintainers ask affected users to contact their AV/EDR vendor, obtain a support ticket, and share the ticket ID.
- astral-sh/uv#10428 — **Doc and install: avoid security issue on Windows** (open issue). A later comment reports that Netskope detects and blocks uv as malware on every release, particularly the x64 build. The main issue is adjacent rather than canonical because it primarily discusses security software blocking the PowerShell installation command.
- astral-sh/uv#10336 — **Sign published executables for Windows** (closed issue). This tracked signing Windows release executables and was closed when a maintainer confirmed that published binaries are signed starting with uv 0.12.12. Maintainers caution that signatures do not prevent all vendor false positives, so this does not confirm the detection’s root cause or guarantee an immediate resolution.
- astral-sh/uv#18280 — **Add code signing of release binaries via `cargo-code-sign`** (open pull request). This is an earlier implementation of release-binary signing for Windows and macOS and remains open. Its description predates production signing and states that release credentials were not configured at that time; the canonical tracker now confirms published signing separately as of uv 0.12.12.

## Search evidence

Literal searches covered `Netskope`, uv 0.12.6 and 0.12.7, virus detection and removal, and installation blocking. Conceptual searches covered antivirus/AV, EDR, quarantine, malware and false-positive terminology, unsigned executables, SmartScreen, and Windows code signing. Fix-oriented searches included open and closed issues and open, closed, and merged pull requests, with inspection of comments and referenced issue chains.

astral-sh/uv#20567 and related Windows runtime file-lock reports were inspected but ruled out as less direct. They concern AV/EDR products transiently locking trampoline or PE-resource files during uv operations, whereas astral-sh/uv#21336 reports Netskope removing or blocking a uv release during installation.
