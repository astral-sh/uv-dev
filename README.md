# No migration path for native-tls -> system-certs with pinned environments

Issue: astral-sh/uv#21035

Classification: enhancement

## Summary

The reporter uses one global `uv.toml` with both a current global uv installation and uv versions
pinned inside reproducible virtual environments. They need system certificates on Windows. A
pre-0.11 uv binary does not know the newer `system-certs` setting, while current uv accepts the
older `native-tls` setting but emits a deprecation warning saying that it will be removed. As a
result, there is no single configuration key that is both understood by older pinned versions and
warning-free in current versions.

No existing issue was found that tracks this shared-configuration migration gap. The closest
history is astral-sh/uv#18550, which introduced `system-certs` while retaining `native-tls` as an
identical compatibility alias, and astral-sh/uv#18705, which intentionally added the deprecation
warning. Neither discusses a global configuration file shared with older uv binaries.

## Draft response

Thanks for calling out the shared-configuration case. astral-sh/uv#18550 introduced
`system-certs` in uv 0.11 while retaining `native-tls` with identical behavior, and
astral-sh/uv#18705 added the deprecation warning. That means there is currently no single
`uv.toml` key that is both recognized by pre-0.11 uv and warning-free in current uv:
`native-tls` remains the compatible key, but current versions warn on it.

We'll treat this as an enhancement to the migration behavior and use this issue to decide whether
the legacy alias and warning policy need to change before `native-tls` can be removed.

## Classification

This is an enhancement rather than a bug or question. The current implementation behaves as
designed: `system-certs` is the new name, `native-tls` remains a behaviorally identical legacy
alias, and use of that alias deliberately emits a deprecation warning. No current certificate
behavior is reported broken, and the feared removal has not happened. The missing capability is a
warning-free, backward-compatible migration path for configuration consumed by multiple uv
versions.

It is not a duplicate. The originating and follow-up pull requests establish why the two names and
the warning exist, but neither tracks compatibility with pinned pre-0.11 consumers of a shared
global configuration.

## Related

### astral-sh/uv#18550 — Upgrade reqwest to 0.13 (merged)

This is the originating change. It introduced `--system-certs` and the `system-certs` setting,
deprecated the `native-tls` name, and explicitly retained `native-tls` with behavior identical to
`system-certs`. The uv 0.11.0 changelog confirms that the rename was intended to clarify that uv
uses rustls rather than the `native-tls` TLS implementation. It does not provide version-aware
configuration or otherwise solve the older-binary compatibility case.

### astral-sh/uv#18705 — Mark `--native-tls` and `UV_NATIVE_TLS` as deprecated (merged)

This follow-up closed astral-sh/uv#18689 and deliberately added deprecation warnings. Maintainer
comments explicitly say that a warning was intended and then added. It confirms that the warning
is intentional, but its discussion does not consider pinned uv versions reading the same global
`uv.toml`, so it is causal context rather than a duplicate.

### Inspected but not included

astral-sh/uv#18689 requested documentation that `UV_NATIVE_TLS` was deprecated and was closed by
astral-sh/uv#18705. It is adjacent but not a canonical discussion of configuration migration.
astral-sh/uv#17427 and the superseded astral-sh/uv#17543 concern the reqwest/TLS-stack upgrade and
certificate verification behavior, not coexistence of old and new configuration keys.

## Search and evidence

Literal searches covered `native-tls`, `system-certs`, `UV_NATIVE_TLS`, the deprecation wording,
unknown-field configuration failures, `uv.toml`, and pinned versions. Conceptual searches covered
shared or global configuration, older-version and backward compatibility, ignored unknown
settings, version-aware migration, and configuration deprecation. Searches included open and
closed issues plus open, closed-unmerged, and merged pull requests. The strongest candidates,
their comments, and their referenced issue and pull-request chains were inspected.

Repository source supports the report's compatibility premise: current settings resolution warns
when `native-tls` is supplied by command line, environment, or configuration, then treats it as a
fallback alias for `system-certs`. Integration snapshots assert the configuration warning. The
0.11.0 changelog dates the introduction of `system-certs` and states that `native-tls` remains
usable with identical behavior.
