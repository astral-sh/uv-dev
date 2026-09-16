# Intermittent SIGABRT on aarch64 (GCP Axion / AWS Graviton) during package downloads - AWS-LC RNDR no-retry

Issue: astral-sh/uv#21743

Classification: bug

## Summary

uv 0.11.8 is reported to terminate with SIGABRT (exit 134) during cold-cache HTTPS
downloads on aarch64 Linux. The failure affects `uv run`, `uv sync`, and `uv pip install`,
has been observed with both glibc and musl release binaries, and does not occur once the
required artifacts are cached. A debug build produced a backtrace from AWS-LC's
`rand_bytes_core` through `RAND_bytes`, `aws_lc_rs::rand::fill`, rustls session-ID generation,
and the reqwest connection path.

The underlying mechanism is confirmed upstream. aws/aws-lc#3453 identifies a transient failed
RNDR read on aarch64 CPUs with FEAT_RNG as the path that causes `RAND_bytes` to abort. AWS-LC
accepted `OPENSSL_armcap=~0x20000`, which disables the RNDR capability bit, as a temporary
workaround. The reporter's result of zero aborts across 380 builds is additional evidence for
that path.

AWS-LC merged a bounded RNDR retry in aws/aws-lc#3475 on 2026-09-15. That merge occurred after
AWS-LC v5.9.0, and the upstream maintainer said the next release should include it. The latest
aws-lc-rs release, v1.18.1, also predates the fix. This checkout currently resolves aws-lc-rs
1.18.0 and aws-lc-sys 0.44.0, so there is not yet a released fixed dependency for uv to adopt.

## Draft response

Thanks for the concrete backtrace and workaround data. This matches the failure tracked in
aws/aws-lc#3453 and the aws-lc-rs propagation report aws/aws-lc-rs#1233. AWS-LC merged bounded
RNDR retries in aws/aws-lc#3475 on 2026-09-15, but its latest release predates that merge;
upstream says the fix should be included in the next release. Upstream also confirmed that
`OPENSSL_armcap=~0x20000` should work as a temporary workaround.

Let's keep astral-sh/uv#21743 open to track an aws-lc-rs/aws-lc-sys release containing the fix
and the corresponding uv dependency update, and consider adding a temporary workaround note
while that propagates. astral-sh/uv#21537 is related alternative-backend work, but it is a
compile-time option for downstream builds and does not change the official rustls-based binaries.

## Classification

This is a bug because an ordinary package download can terminate the entire uv process, and the
responsible AWS-LC behavior is confirmed by the upstream issue and merged fix. The requests for
documentation and a provider or RNG control are possible mitigations, but they do not change the
classification of the reported crash.

This is not a duplicate. No existing uv issue or pull request tracks the same intermittent
aarch64 RNDR abort. The exact aws-lc-rs report and the closed AWS-LC issue cover the upstream
dependency layers, but they do not track propagation into uv releases or uv-specific workaround
documentation. This is also not a regression of an earlier uv fix: astral-sh/uv#21158 addressed
a different AWS-LC-related crash on RISC-V.

## Related

- aws/aws-lc-rs#1233 — Open. This is the exact aws-lc-rs propagation report: it has the same
  backtrace, platforms, intermittent rate, workaround, and request to consume the AWS-LC fix.
- aws/aws-lc#3453 — Closed. This is the canonical upstream defect. Maintainer discussion confirms
  the RNDR failure path and acknowledges `OPENSSL_armcap=~0x20000` as a viable workaround.
- aws/aws-lc#3475 — Merged. This adds the bounded RNDR retry that directly fixes the abort. It
  merged after the currently published AWS-LC and aws-lc-rs releases.
- astral-sh/uv#21537 — Open. This proposes an optional native-TLS/OpenSSL backend for downstream
  builds, which is adjacent to the requested provider control. It does not provide a runtime RNG
  switch or alter official rustls-based binaries.
- astral-sh/uv#21158 — Merged. This is the closest earlier uv AWS-LC HTTPS crash fix and explains
  the aws-lc-rs 1.18.0/aws-lc-sys 0.44.0 versions in the current checkout. Its deterministic
  RISC-V musl SIGSEGV came from a toolchain miscompilation; the new report is an intermittent
  aarch64 SIGABRT on both musl and glibc caused by RNDR failure.

## Search coverage and ruled-out candidates

Authenticated GitHub searches covered open and closed issues and open, closed, and merged pull
requests. Literal terms included `SIGABRT`, `exit 134`, `RAND_bytes`, `RNDR`,
`OPENSSL_armcap`, `aws-lc-rs`, `SessionId`, `Axion`, and `Graviton`. Conceptual searches covered
aarch64/arm64 download crashes, cold-cache and HTTPS-handshake failures, core dumps, random-number
generation, crypto/TLS providers, native TLS, getrandom, and dependency-update fixes. Exact terms
other than the new report found no earlier uv match.

The most plausible uv false positives were inspected with their comments. astral-sh/uv#21337 is
a macOS subprocess-stdio SIGABRT with a different trigger. astral-sh/uv#18890 is an arm64 uv 0.11
TLS failure, but it is a deterministic certificate-extension panic rather than a random-generation
abort. astral-sh/uv#20632 requests release debug symbols after a separate intermittent Windows
AWS-LC crash. The broader provider discussions in astral-sh/uv#11595 and astral-sh/uv#8838 concern
system OpenSSL and certificate-store behavior, not this RNDR defect.
