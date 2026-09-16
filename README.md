# Intermittent SIGABRT on aarch64 (GCP Axion / AWS Graviton) during package downloads - AWS-LC RNDR no-retry

Issue: astral-sh/uv#21743

Classification: bug

## Summary

uv 0.11.8 is reported to terminate with SIGABRT (exit 134) during cold-cache HTTPS
downloads on aarch64 Linux. The report covers `uv run`, `uv sync`, and `uv pip install`,
both glibc and musl release binaries, and Ubuntu 24.04 on GCP Axion. The reported debug
backtrace runs from AWS-LC's random-byte generation through `RAND_bytes`,
`aws_lc_rs::rand::fill`, rustls session-ID generation, and the reqwest connection path.

This triage did not directly reproduce the abort because the available runner and installed uv
binary are x86_64 and therefore cannot execute the reported aarch64 RNDR path. A bounded
cold-cache fixture completed successfully on x86_64 with both the affected uv version and the
currently installed version; those successes do not contradict a CPU-specific intermittent
failure.

The upstream record strongly supports the reported diagnosis without constituting a uv runtime
reproduction. aws/aws-lc#3453 tracks transient RNDR failure on aarch64, and
aws/aws-lc#3475 adds bounded retries with a fault-injected AWS-LC test. The pull request explicitly
notes that a controllable hardware RNG failure cannot easily be induced on real hardware. It
merged on 2026-09-15 after AWS-LC v5.9.0 was published. The latest aws-lc-rs release, v1.18.1,
also predates the fix. uv 0.11.8 resolved aws-lc-rs 1.16.2 and aws-lc-sys 0.39.0; the current
checkout resolves aws-lc-rs 1.18.0 and aws-lc-sys 0.44.0.

## Reproduction

Outcome: `needs_more_information`.

The available environment was Linux x86_64 on an Azure runner, with uv 0.12.13
(`x86_64-unknown-linux-gnu`) and CPython 3.12.3. The public x86_64 uv 0.11.8 package was acquired
through the installed uv executable into a temporary tool directory. All caches, targets, logs,
and tool files were isolated under `/tmp/uv-21743.EplcCx`; neither the checkout nor existing uv
state was changed.

The reconstructed operation was a fresh-cache HTTPS resolution and wheel download from public
PyPI on every iteration:

```console
UV_TOOL_DIR="$CASE_DIR/tools" uv --no-config --cache-dir "$CASE_DIR/tool-cache" \
  tool run --from 'uv==0.11.8' uv --no-config \
  --cache-dir "$CASE_DIR/cache-$ITERATION" pip install \
  --python "$(command -v python3)" --target "$CASE_DIR/target-$ITERATION" \
  'iniconfig==2.0.0'
```

uv 0.11.8 completed 20 of 20 isolated cold-cache installs with exit code 0 and no signal.
The installed uv 0.12.13 completed 5 of 5 equivalent installs with exit code 0 and no signal.
An earlier historical-version setup attempt failed before executing uv 0.11.8 because an invalid
outer `uv tool install` option was used; those setup failures were discarded and are not crash
evidence.

This is not a meaningful negative result for astral-sh/uv#21743: x86_64 does not expose the
aarch64 FEAT_RNG/RNDR behavior named in the report, and a normal HTTPS fixture cannot force a
transient RNDR read failure. Direct reproduction requires an aarch64 FEAT_RNG host exhibiting the
failure (for example, an affected GCP Axion physical host), the uv 0.11.8 glibc or musl binary,
and enough cold-cache HTTPS handshakes to observe the intermittent event. A paired run with and
without `OPENSSL_armcap=~0x20000` would test the reported mitigation.

For a reporter-provided repeatable fixture, maintainers still need the exact uv command, a minimal
requirements file or lockfile (or the package set), whether the index is public PyPI or a custom
index/proxy, the exact uv artifact and `uv --version` output, CPU feature information, and attempt
and abort counts for paired cold-cache runs. No credentials or private index URLs are needed.

Exact searches for RNDR, `OPENSSL_armcap`, `RAND_bytes`, SIGABRT, exit 134, and AWS-LC found no
test covering this failure in `crates/uv/tests/` or `crates/uv-client/tests/it/`.
`crates/uv-client/tests/it/ssl_certs.rs::test_webpki_roots_trusts_pypi` covers a successful public
PyPI TLS connection, while the TLS retry tests in that file inject protocol or certificate errors
after networking begins. `crates/uv/tests/it/network.rs::connect_timeout_index` and
`connect_timeout_stream` cover HTTPS connection timeouts. None injects an AWS-LC entropy-source
failure or runs the affected hardware path.

## Draft response

Thanks for the concrete backtrace and workaround data. The upstream AWS-LC issue and merged retry
fix strongly support this diagnosis, but we could not reproduce the uv abort on the available
x86_64 runner because it cannot exercise aarch64 RNDR. Twenty isolated cold-cache installs with
uv 0.11.8 succeeded on x86_64, which is not evidence against the hardware-specific report.

Could you provide the exact command and a minimal dependency or lockfile fixture, identify the
exact uv artifact, and include paired attempt/abort counts with and without
`OPENSSL_armcap=~0x20000` from an affected host? Please omit credentials and private index details.
aws/aws-lc#3475 merged the bounded RNDR retry, but the current AWS-LC and aws-lc-rs releases predate
it, so astral-sh/uv#21743 can track propagation into an aws-lc-rs/aws-lc-sys release and then uv.

## Classification

The classification remains bug because an ordinary package download is reported to terminate the
entire uv process, and AWS-LC has independently fixed the same transient RNDR failure mode. The uv
crash itself remains unobserved in this triage, so the upstream issue, code change, and related
report are supporting evidence rather than a claimed reproduction or independently confirmed uv
root cause.

This is not currently identified as a duplicate. No existing uv issue or pull request in the
preserved search record tracks the same intermittent aarch64 RNDR abort. This is also not a
regression of an earlier uv fix: astral-sh/uv#21158 addressed a different AWS-LC-related crash on
RISC-V.

## Related

- aws/aws-lc-rs#1233 — Open. This is the matching aws-lc-rs propagation report, with the same
  backtrace, platforms, intermittent rate, workaround, and request to consume the AWS-LC fix.
- aws/aws-lc#3453 — Closed. This is the canonical upstream report for transient RNDR failure on
  aarch64.
- aws/aws-lc#3475 — Merged. This adds a bounded RNDR retry and an injectable unit test for retry
  behavior. It merged after the currently published AWS-LC and aws-lc-rs releases.
- astral-sh/uv#21537 — Open. This proposes an optional native-TLS/OpenSSL backend for downstream
  builds. It does not provide a runtime RNG switch or alter official rustls-based binaries.
- astral-sh/uv#21158 — Merged. This addressed a deterministic RISC-V musl SIGSEGV attributed to a
  toolchain miscompilation; the new report concerns an intermittent aarch64 SIGABRT on both musl
  and glibc.

## Search coverage and ruled-out candidates

The preserved issue context records searches of open and closed uv issues and pull requests for
`SIGABRT`, exit 134, `RAND_bytes`, RNDR, `OPENSSL_armcap`, aws-lc-rs, `SessionId`, Axion, and
Graviton, plus conceptual searches for arm64 download crashes, cold-cache HTTPS failures, random
number generation, TLS providers, and related dependency updates.

The closest uv candidates remain distinct: astral-sh/uv#21337 is a macOS subprocess-stdio SIGABRT;
astral-sh/uv#18890 is a deterministic arm64 certificate-extension panic; astral-sh/uv#20632 is a
separate intermittent Windows AWS-LC crash; and astral-sh/uv#11595 and astral-sh/uv#8838 concern
system OpenSSL and certificate-store behavior rather than this RNDR failure mode.
