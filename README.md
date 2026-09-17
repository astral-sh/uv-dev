# Remove the annoying message "`UV_NATIVE_TLS` environment variable is deprecated" when both variables are defined

Issue: astral-sh/uv#21774

Classification: enhancement

## Summary

The reporter sets both `UV_NATIVE_TLS` and `UV_SYSTEM_CERTS` so the same environment works with
multiple uv versions. uv 0.12.15 emits the `UV_NATIVE_TLS` deprecation warning on every invocation,
even though the replacement variable is also present, and the reporter requests that this warning
be suppressed for that compatibility setup.

No duplicate was found. astral-sh/uv#18550 introduced `UV_SYSTEM_CERTS` as the clearer replacement
while retaining `UV_NATIVE_TLS` as a legacy alias. astral-sh/uv#18705 subsequently added the exact
warning reported here. Current source emits it whenever `UV_NATIVE_TLS` is present, before resolving
which certificate setting is effective, with no check for `UV_SYSTEM_CERTS`.

## Draft response

Thanks. astral-sh/uv#18705 intentionally added this warning after astral-sh/uv#18550 introduced
`UV_SYSTEM_CERTS` as the clearer replacement. The current settings code warns whenever
`UV_NATIVE_TLS` is present, before resolving whether `UV_SYSTEM_CERTS` already supplies the
effective setting. Keeping both variables for compatibility with older uv versions is a valid
migration case. We can use this issue to consider suppressing the warning when the replacement
variable makes the legacy alias redundant, while retaining it for combinations where
`UV_NATIVE_TLS` can still affect behavior.

## Classification

This is an enhancement. The warning is intentional deprecation behavior added by
astral-sh/uv#18705, rather than a regression or a feature that fails to work. The requested change
would refine that behavior for mixed-version environments by suppressing a warning when the
replacement setting already makes the legacy alias redundant. No existing issue or pull request
tracks this special case, so this is not a duplicate.

The distinction between variables merely being present and their Boolean values matters to a
future implementation: suppression should not hide the warning in combinations where
`UV_NATIVE_TLS` can still determine the resolved certificate behavior.

## Related

- astral-sh/uv#18705 (merged pull request), **Mark `--native-tls` and `UV_NATIVE_TLS` as
  deprecated** — This is the direct origin of the exact warning. Maintainer comments explicitly
  requested and then confirmed adding the warning, and its patch warns whenever `UV_NATIVE_TLS` is
  present without checking `UV_SYSTEM_CERTS`.
- astral-sh/uv#18689 (closed issue), **Document UV_NATIVE_TLS as being deprecated** — This requested
  deprecating `UV_NATIVE_TLS` and was closed by astral-sh/uv#18705. It establishes the warning's
  intended migration purpose but does not discuss suppressing it when both variables are set.
- astral-sh/uv#18550 (merged pull request), **Upgrade reqwest to 0.13** — This introduced
  `UV_SYSTEM_CERTS` as the clearer replacement while retaining `UV_NATIVE_TLS` as a legacy alias,
  which directly explains the reporter's mixed-version compatibility setup.

## Supporting evidence

Literal searches covered the exact warning text, `UV_NATIVE_TLS`, `UV_SYSTEM_CERTS`, and reports
mentioning both variables. Conceptual searches covered repeated deprecation diagnostics,
native-versus-system certificate terminology, backward compatibility across uv versions, warning
suppression when a replacement setting is present, and the TLS migration and subsequent fixes.
Searches included open and closed issues and open, closed, and merged pull requests.

The linked history from astral-sh/uv#17427 through the superseded astral-sh/uv#17543 and merged
astral-sh/uv#18550 was inspected, as were astral-sh/uv#18689 and astral-sh/uv#18705. The warning was
documented in the uv 0.11.9 changelog, so its appearance in 0.12.15 is not a regression.

astral-sh/uv#12982 was a plausible identifier match but was ruled out: it concerns precedence
between `UV_NATIVE_TLS` and `SSL_CERT_FILE` when certificate verification fails, not deprecation
warning emission. No existing issue or pull request requests the same conditional suppression.
