# Request to add s390x and ppc64le as `--python-platform` targets

Issue: astral-sh/uv#21299

Classification: enhancement

## Summary

The reporter uses `uv pip compile --python-platform` to resolve dependency closures for wheels on a
PEP 503 index. The existing x86_64 and aarch64 GNU/Linux target triples work, but uv 0.12.6 rejects
`s390x-unknown-linux-gnu` and `powerpc64le-unknown-linux-gnu` during CLI value parsing. They request
those two cross-resolution targets and, ideally, corresponding explicit manylinux selectors such as
`s390x-manylinux_2_17` and `ppc64le-manylinux_2_17`.

No existing issue or pull request was found that tracks adding these two architectures to
`--python-platform`. The closest discussions expanded the same static target list with additional
manylinux policies for x86_64 and aarch64. Older S390x and PowerPC work concerned building uv's own
release wheels, not selecting those architectures while resolving Python packages.

A repository member has since said the request is reasonable. This is a positive maintainer signal
for the enhancement, but the discussion does not yet decide the accepted spellings, default
manylinux compatibility floor, exact variant set, or implementation plan.

## Draft response

Thanks. The accepted `--python-platform` values come from uv's explicit `TargetTriple`
configuration enum. It currently has GNU/Linux targets for x86_64, aarch64, and riscv64, but not
s390x or ppc64le; the lower-level platform-tag code does already recognize both architectures.
Existing target-expansion work such as astral-sh/uv#4966 and astral-sh/uv#9234 only added manylinux
policies for the existing architectures, so this is a distinct enhancement request.

The next step is maintainer agreement on the target spellings and default manylinux compatibility
floor before implementation adds the enum mappings, marker environment behavior, schema/help
entries, and integration coverage. Please hold off on a pull request until that scope is confirmed.

## Classification

This is an enhancement. The CLI and configuration schema derive their accepted values from the
explicit `TargetTriple` enum in `crates/uv-configuration/src/target_triple.rs`. Current source
defines GNU/Linux target variants for x86_64, aarch64, and riscv64, but not S390x or little-endian
PowerPC. The documented/configured value set therefore does not promise the rejected inputs.
Adding them would extend supported functionality rather than correct a regression.

The reporter's observation about the lower-level layer is supported by the source:
`crates/uv-platform-tags/src/platform.rs` recognizes `Arch::S390X` and
`Arch::Powerpc64Le`, and `platform_tag.rs` contains architecture-specific compatibility helpers.
That makes the requested wiring plausible, but it does not establish that the higher-level target
selection, marker-environment mapping, naming, and manylinux baseline are already implemented.

No same-request tracker was found, so the issue is not a duplicate. The repository's earlier
manylinux target-list request, astral-sh/uv#4966, was also classified as an enhancement.

## Maintainer status

Charlie Marsh responded that the request is reasonable. Treat this as initial acceptance of the
feature direction, not a completed design decision: no target names or compatibility baselines were
confirmed, and the comment did not explicitly invite an implementation pull request. The remaining
next step is to agree on that scope before implementation.

## Related

- astral-sh/uv#4966 — Closed issue and closest conceptual CLI-target discussion. It addressed the
  static `TargetTriple`/Clap value list and requested broader `manylinux_x_y` selectors. It was
  resolved by enumerating additional policies for x86_64 and aarch64, not by adding s390x or
  ppc64le architectures.
- astral-sh/uv#9234 — Merged pull request and closest implementation precedent. It expanded the
  same `TargetTriple` enum, CLI help, and schema with explicit manylinux values through glibc 2.40.
  Its variants remain limited to existing x86_64 and aarch64 architectures.
- astral-sh/uv#4956 — Closed earlier request to expand accepted `--python-platform` values,
  specifically for `aarch64-manylinux_2_31` and potentially arbitrary PEP 600 glibc versions. It
  confirms the same feature surface but does not cover the requested CPU architectures.
- astral-sh/uv#1015 — Closed issue and the only strong historical match on both architecture names.
  It concerned building and publishing uv's own S390x and PowerPC wheels and was closed by
  PowerPC release-build work in astral-sh/uv#1017, not `uv pip compile` target selection.

## Search and supporting evidence

Searches covered open and closed issues and open, closed, and merged pull requests. Literal queries
included `s390x`, `ppc64le`, `powerpc64le`, the two requested target triples, `invalid value`, and
`--python-platform`. Conceptual and fix-oriented queries included PowerPC, cross-platform
resolution, target triples, platform tags, unsupported architectures, PEP 600, and manylinux
selectors. The strongest candidates' comments, timelines, closure links, and referenced pull
requests were inspected.

The astral-sh/uv#4956 chain led to astral-sh/uv#4966 and astral-sh/uv#9234. The
astral-sh/uv#1015 chain led to astral-sh/uv#1017. Manylinux alias work in astral-sh/uv#10210 and
astral-sh/uv#10217 was also inspected; it only added manylinux2014 aliases for x86_64 and aarch64.
The especially plausible open astral-sh/uv#7957 was ruled out because it asks for runnable
cross-platform virtual environments and cross-build metadata, which is substantially broader than
adding two wheel-resolution targets to the existing flag.
