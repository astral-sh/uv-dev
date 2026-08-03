# Kaspersky System Watcher flags uv.exe as PDM:Trojan.Win32.Generic

Issue: astral-sh/uv#20906
Classification: duplicate

## Summary

The exact Kaspersky signature is new, but the underlying Windows antivirus false-positive problem is already tracked by astral-sh/uv#10336 and its open signing implementation astral-sh/uv#18280; prior vendor-specific reports show closely matching behavior.

## Classification

The report is another instance of the recurring Windows antivirus false-positive problem already centralized in open astral-sh/uv#10336, with an active mitigation in astral-sh/uv#18280. Repository history supports that such heuristic alarms are external false positives, but it does not confirm why Kaspersky flags uv 0.12.1 or that code signing will eliminate this specific behavioral detection.

## Related

- https://github.com/astral-sh/uv/issues/10336 (open issue): Sign published executables for Windows
  astral-sh/uv#10336 is the active canonical discussion for signing Windows releases to reduce recurring antivirus reports. It explicitly tracks this problem class, although signing is only a proposed mitigation and is not confirmed to prevent Kaspersky's behavioral detection.
- https://github.com/astral-sh/uv/pull/18280 (open pull request): Add code signing of release binaries via `cargo-code-sign`
  astral-sh/uv#18280 implements the Windows code-signing work tracked by astral-sh/uv#10336. It remains unfinished and does not establish the cause of PDM:Trojan.Win32.Generic.
- https://github.com/astral-sh/uv/issues/13553 (closed issue): Bitdefender is detecting UV as malware
  astral-sh/uv#13553 is the closest prior vendor-specific report: a third-party antivirus heuristically identified an official Windows uv release as malware. Maintainers classified it as a vendor false positive, and the vendor later resolved it after submission.
- https://github.com/astral-sh/uv/issues/17344 (closed issue): The latest version 0.9.22 doesn't work on Windows
  astral-sh/uv#17344 documents the same observable release-binary blocking pattern and maintainer guidance that heuristic detections can disappear after updated definitions or reputation analysis. It differs because it involved Microsoft Defender, mostly uvw.exe/uvx.exe, and uv 0.9.22.
