# Checksum authority prototype

This experimental service lets `uv` authenticate downloaded Python package archives against a
separately administered checksum catalog. It is an exploration of a third-party checksum authority,
not a production service or an implementation of the proposed PyPI transparency standard.

The service never fetches packages or admits hashes on demand. An administrator explicitly admits
local archive bytes. Existing records cannot be replaced through the catalog API. The running
service signs its catalog at startup and exposes only a read API.

## Try it

Build the two executables from the workspace root:

```console
cargo build -p uv -p uv-checksum-authority-service
```

Generate a signing key. The command prints the hexadecimal public key; keep the signing-key file
private and distribute the public key independently of the package index.

```console
target/debug/uv-checksum-authority-service keygen --signing-key authority.key
```

Admit a wheel or source archive whose bytes you already trust. The source identifies the package
index, not its artifact CDN. Use the exact configured index URL; trailing slashes are normalized.
For a direct archive requirement, use the archive URL itself as the source.

```console
target/debug/uv-checksum-authority-service add \
  --catalog catalog.json \
  --source https://pypi.org/simple \
  ./example-1.0.0-py3-none-any.whl
target/debug/uv-checksum-authority-service serve \
  --catalog catalog.json \
  --signing-key authority.key
```

Configure the client with the public key printed by `keygen`:

```console
UV_CHECKSUM_AUTHORITY=http://127.0.0.1:8080 \
UV_CHECKSUM_AUTHORITY_KEY=<hexadecimal-public-key> \
target/debug/uv pip install example
```

The equivalent experimental CLI options are `--checksum-authority` and `--checksum-authority-key`.
Both must be supplied. Remote authorities require HTTPS; plain HTTP is accepted only on loopback for
development. Put the service behind a TLS reverse proxy for remote use. The server's default
listener is loopback-only.

## Protocol

`GET /v1/checksum?source=<url>&filename=<filename>` returns a JSON object containing `payload` and
`signature`, both standard base64. The decoded payload is UTF-8 JSON:

```json
{
  "artifact": {
    "source": "https://pypi.org/simple",
    "filename": "example-1.0.0-py3-none-any.whl"
  },
  "sha256": "<64 lowercase hexadecimal characters>"
}
```

The Ed25519 signature covers the ASCII bytes `uv-checksum-authority/v1\n` followed by the exact
decoded payload bytes. Clients verify the signature, validate the record, and require an exact
identity match. `404` means the artifact has not been admitted. Neither missing records nor
connection failures permit an unverified installation. Credentials, fragments, and query strings are
not sent to the authority; identities containing query strings are unsupported.

The SHA-256 digest covers the complete original archive, before extraction. It is independent of
hashes supplied by an index or lockfile: those checks continue to apply. The service's `/health`
endpoint reports readiness of the loaded catalog.

## Prototype boundaries

- Authority mode uses a fresh temporary `uv` cache. Existing cache entries lack verification
  receipts, so they cannot establish that an authority approved their contents. Persistent verified
  caching, offline operation, and revocation need a separate design.
- Wheel metadata is read from the complete verified wheel. PEP 658 sidecars, range reads, and
  alternate compressed-wheel representations are not trusted in this mode.
- Remote wheels and source archives are checked, including archives fetched while resolving and
  installing build dependencies. Local projects, local archives, Git dependencies, already installed
  packages, Python interpreter downloads, and code fetched by a build backend are outside this
  prototype's authority policy.
- An authority-enabled install checks each archive it actually downloads. It does not certify every
  alternative artifact written to a universal lockfile, make resolution reproducible, or prove that
  an admitted artifact is harmless.
- Catalog admission is an offline administrative operation with an advisory writer lock. The
  prototype has no crawler, approval workflow, revocation mechanism, authentication layer, key
  rotation, transparency log, independent witnesses, or availability guarantees. Do not deploy it as
  an organization-wide security control without those decisions and an operational owner.

The long-term protocol should be considered alongside
[PyPI Transparency](https://pytransparency.dev/), rather than treating this experimental API as a
packaging standard.
