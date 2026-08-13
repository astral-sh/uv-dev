# TLS certificates

uv uses TLS for secure connections to package indexes and other HTTPS servers. TLS certificates
verify the identity of these servers and help prevent intercepted connections.

## TLS backend

uv uses [`rustls`](https://github.com/rustls/rustls), a memory-safe TLS implementation written in
Rust, with [`aws-lc-rs`](https://github.com/aws/aws-lc-rs) as the cryptography provider.

uv supports the following X.509 certificate signature algorithms:

- ECDSA (P-256, P-384, P-521) with SHA-256, SHA-384, or SHA-512
- Ed25519
- RSA PKCS#1 v1.5 (2048–8192 bit) with SHA-256, SHA-384, or SHA-512
- RSA-PSS (2048–8192 bit) with SHA-256, SHA-384, or SHA-512

## System certificates

By default, uv verifies TLS connections with bundled Mozilla root certificates. A system certificate
store can provide a required corporate trust root. For example, a mandatory proxy can require one.

To use system certificates, pass the [`--system-certs`](../../reference/cli.md#uv) flag, set the
[`UV_SYSTEM_CERTS`](../../reference/environment.md#uv_system_certs) environment variable to `true`,
or set [`system-certs = true`](../../reference/settings.md#system-certs) in `uv.toml`.

When uv uses system certificates,
[`rustls-platform-verifier`](https://github.com/rustls/rustls-platform-verifier) verifies them with
the operating system's certificate verifier.

## Custom certificates

To use custom CA certificates, set [`SSL_CERT_FILE`](../../reference/environment.md#ssl_cert_file)
to the path of a PEM-encoded certificate bundle, such as `certs.pem` or `ca-bundle.crt`.
Alternatively, set [`SSL_CERT_DIR`](../../reference/environment.md#ssl_cert_dir) to one or more
directories that contain PEM-encoded certificate files. Separate multiple directories with `:` on
Unix or `;` on Windows.

!!! note

    For a single `uv pip` invocation, pass [`--cert`](../../reference/cli.md#uv-pip) with the path
    to a PEM-encoded certificate bundle. The bundle replaces uv's default certificate source for
    that invocation.

Certificates usually have `.pem`, `.crt`, or `.cer` extensions. However, uv attempts to read a
certificate from every regular file in `SSL_CERT_DIR`.

uv ignores files that it cannot parse as PEM certificates. It resolves symlinks and ignores dangling
symlinks.

uv does not support DER-encoded files.

Non-empty values for these environment variables **replace** the default certificate source. uv
trusts only the specified certificates. If a specified file or directory does not exist or contains
no valid certificates, uv does not trust any default certificates.

`SSL_CERT_FILE` can point to a single certificate or a bundle containing multiple certificates.
`SSL_CERT_DIR` can include multiple directories. uv loads all valid certificates from each
directory.

To use client certificate authentication (mTLS), set
[`SSL_CLIENT_CERT`](../../reference/environment.md#ssl_client_cert) to a PEM-formatted file. The
file must contain the certificate followed by the private key.

## Insecure hosts

To trust a self-signed certificate or disable certificate verification for specific hosts, use
[`allow-insecure-host`](../../reference/settings.md#allow-insecure-host). For example, add the
following to `pyproject.toml` to allow insecure connections to `example.com`:

```toml
[tool.uv]
allow-insecure-host = ["example.com"]
```

`allow-insecure-host` accepts a hostname, such as `localhost`, or a hostname and port, such as
`localhost:8080`. It applies only to HTTPS connections because HTTP connections are already
insecure.

Use `allow-insecure-host` only in trusted environments. Connections without certificate verification
can expose credentials and other sensitive data.
