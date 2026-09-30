# UV builds fine with NetBSD but refuses to install

Issue: astral-sh/uv#22110

Classification: bug

## Summary

On NetBSD 11_STABLE x86_64 with Python 3.14.7 and uv 0.12.21, maturin 1.15.0 successfully builds native wheels for packages including pyreqwest, orjson, and ty, but uv then rejects the wheels as incompatible. The reported wheel tag is `cp314-cp314-netbsd_11_0_STABLE_amd64`; the reporter also demonstrated that an extracted orjson extension loads on the same host.

The closest historical report is astral-sh/uv#21846, where uv rejected a lowercase `netbsd_11_0_stable_amd64` wheel because its generated compatible tag retained uppercase `STABLE`. Merged fix astral-sh/uv#21853 lowercased generated NetBSD and other BSD-like platform tags and shipped in uv 0.12.18. uv 0.12.21 contains that fix.

The new report has the inverse casing. The 0.12.21 source generates the compatible platform tag as lowercase, while `PlatformTag::from_str` stores the `ReleaseArch` suffix from a wheel filename without lowercasing it. Consequently, maturin's uppercase `netbsd_11_0_STABLE_amd64` tag does not equal uv's lowercase compatible tag. PyPA packaging's `Tag` constructor lowercases incoming interpreter, ABI, and platform strings, which supports treating this as a uv normalization bug rather than an incompatible native artifact.

No open issue or pull request was found that already tracks this post-fix inverse mismatch.

## Draft response

Thanks for the report. uv 0.12.21 already contains the lowercasing change from astral-sh/uv#21853, released in 0.12.18, which fixed astral-sh/uv#21846 for lowercase NetBSD wheel tags. Your reproduction exposes the inverse case: maturin 1.15.0 emits `netbsd_11_0_STABLE_amd64`, while uv generates a lowercase compatible tag but preserves the uppercase suffix when parsing the built wheel, so the tags do not compare equal. This is a regression rather than a duplicate of astral-sh/uv#21846.

Could you share the exact uv command that produced the failure and the output of `python3 -c "import sysconfig; print(sysconfig.get_platform())"`? That will let us cover the platform string reported by your Python and the same build path in a regression test.

## Classification

This is a bug. Rejecting a native wheel that uv just built for the active NetBSD interpreter is incorrect, and the repository source establishes a casing mismatch: generated compatible tags are lowercased by astral-sh/uv#21853, while parsed BSD `ReleaseArch` values retain the wheel's case. Because uv 0.12.21 includes the merged fix and the newly reported uppercase wheel now fails in the opposite direction, this is a regression, not a duplicate of the closed historical report. No open issue or pull request currently centralizes this regression.

## Related

- astral-sh/uv#21846 — Closed issue. This is the direct historical case-handling report on NetBSD 11 and Python 3.14. It observed the same incompatible-platform rejection, but for a lowercase wheel tag compared with uv's then-uppercase compatible tag. It is important history, but the casing direction and release timing differ from astral-sh/uv#22110.
- astral-sh/uv#21853 — Merged pull request. This fixed astral-sh/uv#21846 by lowercasing generated NetBSD, OpenBSD, DragonFly, and Haiku release strings. It merged on 2026-09-20, shipped in uv 0.12.18 on 2026-09-22, and is an ancestor of uv 0.12.21. Its generated-tag normalization, combined with case-preserving parsing of the new uppercase wheel tag, directly explains the regression boundary.

## Search and evidence

The report was decomposed into the NetBSD 11_STABLE/x86_64 trigger, successful PEP 517/maturin builds followed by install-time rejection, the exact `netbsd_11_0_STABLE_amd64` identifier and incompatibility error, and the uv 0.12.21/Python 3.14 release condition. Open and closed issues and open, closed, and merged pull requests were searched separately using literal terms (`NetBSD`, `netbsd_11_0_STABLE`, `not compatible with the current Python`, and `built wheel`) and conceptual terms (`maturin wheel compatibility`, BSD platform tags, sysconfig platform detection, case normalization, and historical fixes).

The search followed astral-sh/uv#21846 through its timeline to merged fix astral-sh/uv#21853 and verified that uv 0.12.18 lists the fix and that the uv 0.12.21 tag contains its merge commit. Closed alternative fixes astral-sh/uv#21848, astral-sh/uv#21852, and astral-sh/uv#21865 were inspected but are superseded proposals for the same historical issue, not separate canonical discussions. astral-sh/uv#18946 (Android API-level mismatch), astral-sh/uv#17635 and astral-sh/uv#18769 (debug/free-threaded ABI tags), and astral-sh/uv#17061 (`cp3-none-any`) share the generic built-wheel incompatibility error but were ruled out because their platforms, tag components, and confirmed mechanisms differ.
