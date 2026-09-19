# uv rejects a wheel package built locally, because it uses case-sensitive compare for NetBSD tag

Issue: astral-sh/uv#21846

Classification: bug

## Summary

On NetBSD 11, `uv pip install` rejects a locally built maturin wheel whose platform tag is `netbsd_11_0_stable_amd64`. uv reports that the current interpreter is compatible with `netbsd_11_0_STABLE_amd64`, so the otherwise matching tags differ only in the case of the release suffix.

Repository source confirms the inconsistency. Compatible-tag construction replaces dots and hyphens for both FreeBSD and NetBSD, but lowercases the release only for FreeBSD. The resulting NetBSD tag therefore retains `STABLE` from the interpreter's platform release and cannot match the lowercase wheel tag.

The closest precedent is astral-sh/uv#15799, which reported the equivalent mismatch on FreeBSD. astral-sh/uv#15829 fixed that case by lowercasing the FreeBSD release and using BSD-native architecture names, but it left NetBSD release casing unchanged. No existing NetBSD-specific issue or pull request was found.

## Draft response

Thanks, this is a bug. NetBSD compatibility-tag construction currently replaces dots and hyphens in the release but, unlike the FreeBSD path fixed in astral-sh/uv#15829, does not lowercase it. That produces `netbsd_11_0_STABLE_amd64` and prevents it from matching the wheel's `netbsd_11_0_stable_amd64` tag. The next step is to apply the same release normalization to NetBSD and add regression coverage for this tag.

## Classification

This is a correctness bug: uv rejects a wheel whose NetBSD platform tag matches the current platform after the expected case normalization. The error output and current source agree on the exact mismatch. This is not a duplicate because astral-sh/uv#15799 and astral-sh/uv#15829 addressed FreeBSD and do not track or fix the NetBSD path. It is also not a regression of that fix; NetBSD lowercasing was absent from the merged change.

## Related

- astral-sh/uv#15799 — Closed issue reporting the exact FreeBSD analogue: locally built wheel metadata contained `freebsd_14_2_release_amd64`, while uv generated an uppercase release and incompatible architecture spelling. Its discussion confirmed the built wheel tag and led to the BSD tag-construction fix.
- astral-sh/uv#15829 — Merged pull request that fixed astral-sh/uv#15799 by lowercasing the FreeBSD release and using BSD-native architecture names. Its patch also handled NetBSD architecture naming but did not lowercase the NetBSD release, making it the closest implementation precedent.

## Search and evidence

Literal searches covered `netbsd`, `netbsd_11_0`, `STABLE_amd64`, the reported incompatibility hint, and the current-platform error. Conceptual searches covered BSD wheel tags, platform mismatch, case sensitivity, lowercase normalization, local wheel compatibility, and tag construction. Fix-oriented searches covered closed issues and merged pull requests, including repository history for the BSD-compatible-tag implementation.

astral-sh/uv#3824 and astral-sh/uv#13713 were inspected but ruled out: they concern libc or interpreter operating-system detection, not wheel platform-tag normalization or comparison. No open or closed NetBSD-specific tracker and no NetBSD-tag pull request was found beyond astral-sh/uv#21846 itself.
