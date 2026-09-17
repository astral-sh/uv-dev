# UV_SYSTEM_CERTS=false can still fall through to UV_NATIVE_TLS=true without warning

Issue: astral-sh/uv#21807

Classification: duplicate

## Summary

The report identifies a mismatch between warning suppression and effective environment-variable
resolution. When `UV_SYSTEM_CERTS=false` and `UV_NATIVE_TLS=true`, uv suppresses the deprecation
warning because the replacement variable is present, but the current resolver only consumes
`UV_SYSTEM_CERTS` when it is true and therefore falls through to the deprecated variable.

Open astral-sh/uv#21805 predates this issue and covers the exact defect. It makes
`UV_SYSTEM_CERTS` authoritative whenever it is set and adds integration coverage for a false
replacement value combined with both a true and an invalid legacy value. Merged
astral-sh/uv#21788 is the warning-suppression change that exposed the mismatch, and closed
astral-sh/uv#21774 provides the compatibility reason users configure both names.

## Draft response

Thanks. This is the exact environment-variable precedence case already addressed by
astral-sh/uv#21805, which was opened before this report. The current code suppresses the warning
whenever `UV_SYSTEM_CERTS` is present, but only gives it precedence during resolution when its value
is true, so false can fall through to `UV_NATIVE_TLS=true`. astral-sh/uv#21805 makes
`UV_SYSTEM_CERTS` authoritative whenever it is set, ignores the deprecated variable in that case,
and adds coverage for both a true and an invalid `UV_NATIVE_TLS` value. We’ll centralize the fix
there.

## Classification

This is a confirmed correctness problem in the checked-out source: the warning condition in
`crates/uv/src/settings.rs` checks whether `environment.system_certs.value` is present, while the
effective-value chain only accepts `Some(true)` before consulting `environment.native_tls.value`.
That makes the reported false/true combination both silent and controlled by the deprecated name.

The classification is `duplicate` because astral-sh/uv#21805 was opened before astral-sh/uv#21807
and already tracks the same underlying behavior, including the false-valued trigger. The pull
request was not created in response to this issue. Its change reads `UV_SYSTEM_CERTS` first, avoids
parsing `UV_NATIVE_TLS` whenever the replacement has a value, and tests `UV_SYSTEM_CERTS=0` with
`UV_NATIVE_TLS=1` and with an invalid legacy value.

## Related

- astral-sh/uv#21805 — Open pull request covering the exact defect. It makes any parsed
  `UV_SYSTEM_CERTS` value authoritative, ignores `UV_NATIVE_TLS` when the replacement is set, and
  adds focused integration coverage for the reported false/true combination.
- astral-sh/uv#21788 — Merged warning-suppression change. It suppresses the warning based on
  replacement-variable presence without changing effective certificate-setting precedence; its
  review discussion explicitly identifies this same corner case.
- astral-sh/uv#21774 — Closed originating warning report. It explains why users set both names to
  support older and newer uv versions, but it does not isolate false-valued replacement precedence.

## Supporting evidence

Literal searches covered `UV_SYSTEM_CERTS`, `UV_NATIVE_TLS`, `UV_SYSTEM_CERTS=false`, both
identifiers together, deprecation-warning text, and ignore/fall-through wording across open and
closed issues and pull requests in every state. Conceptual searches covered TLS-setting precedence,
deprecated/replacement environment variables, dual-setting migration, and warning suppression.
Fix-oriented inspection followed astral-sh/uv#21774 through astral-sh/uv#21788,
astral-sh/uv#21806, and astral-sh/uv#21805, including their comments, reviews, diffs, tests, and
cross-references.

Open astral-sh/uv#21035 was inspected but is not the canonical tracker: it concerns the broader
deprecation timeline for pinned environments rather than false-valued precedence. Merged
astral-sh/uv#21806 is adjacent but configuration-file-only; that path already uses `Option::or`, so
an explicit false `system-certs` value wins over `native-tls` there.
