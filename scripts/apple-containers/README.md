# Apple Containers

These scripts run uv development and reproduction commands with Apple's `container` runtime.

The intended workflow is:

1. Build uv once with cached Cargo state:

   ```console
   $ ./scripts/apple-containers/run-dev.sh cargo build --locked --bin uv
   ```

2. Run trusted reproductions with the locally built uv:

   ```console
   $ ./scripts/apple-containers/run-trusted.sh uv --version
   $ ./scripts/apple-containers/run-trusted.sh uv pip install --system pytest
   ```

3. Run untrusted reproductions with host-only networking and allowlisted egress:

   ```console
   $ ./scripts/apple-containers/run-untrusted.sh sh -c 'uv venv && uv pip install pytest'
   ```

## Scripts

`run-dev.sh` is the development environment. It mounts the repository at `/workspace`, creates
persistent volumes for Cargo and uv caches, and stores the Linux build output in the
`uv-apple-target` volume. Use it for normal Rust development commands.

`run-trusted.sh` is for reproductions that are trusted enough to allow normal network access and a
writable ephemeral container filesystem. It does not mount the repository or home directory by
default. It mounts the `uv-apple-target` volume read-only at `/target` and puts `/target/debug`
first on `PATH`, so it uses the uv built by `run-dev.sh`.

`run-untrusted.sh` is for dependency trees or reproduction cases that may run arbitrary code. It
does not mount the repository or home directory by default. It mounts the `uv-apple-target` volume
read-only, runs as uid/gid `65532`, drops capabilities, uses a read-only container filesystem with
tmpfs scratch space, and joins a host-only Apple container network. HTTP(S) egress is forced through
`egress-proxy.py`.

`egress-proxy.py` is a [PEP 723](https://peps.python.org/pep-0723/) script. The untrusted runner
starts it with `uv run --no-managed-python --no-python-downloads --python python3 --script` on the
host before launching the Apple container.

## Untrusted Egress

The untrusted runner defaults to allowing only PyPI downloads:

```console
$ UV_UNTRUSTED_SHOW_PROXY_LOG=1 \
    ./scripts/apple-containers/run-untrusted.sh sh -c 'uv venv && uv pip install pytest'
```

The default allowlist is:

```text
pypi.org,files.pythonhosted.org,releases.astral.sh
```

The default denylist is:

```text
openai.com,*.openai.com,oaiusercontent.com,*.oaiusercontent.com
```

The proxy also rejects loopback, link-local, multicast, private, and reserved IP addresses after DNS
resolution. This is intended to reduce accidental or malicious access to local and internal
services. It is still a defense-in-depth tool for local reproduction work, not a formal security
boundary for hostile workloads.

The untrusted container intentionally does not include the Apple-container gateway in `NO_PROXY`.
Proxy-aware tools should route HTTP(S) requests through the allowlisting proxy even when the target
is the gateway address, so direct gateway and private-address requests are denied by policy.

## Inputs And Outputs

Trusted and untrusted runs can receive an input directory mounted read-only:

```console
$ UV_UNTRUSTED_INPUT_DIR=/path/to/repro \
    ./scripts/apple-containers/run-untrusted.sh sh -c 'ls -la /input'
```

They can also receive a writable output directory:

```console
$ UV_UNTRUSTED_OUTPUT_DIR=/tmp/uv-output \
    ./scripts/apple-containers/run-untrusted.sh sh -c 'uv --version > /output/version.txt'
```

Use the `UV_TRUSTED_INPUT_DIR` and `UV_TRUSTED_OUTPUT_DIR` variants with `run-trusted.sh`.

## Configuration

Development runner:

| Variable                                   | Default                          |
| ------------------------------------------ | -------------------------------- |
| `UV_APPLE_CONTAINER_IMAGE`                 | `rust:<rust-toolchain>-bookworm` |
| `UV_APPLE_CONTAINER_DNS`                   | `1.1.1.1`                        |
| `UV_APPLE_CONTAINER_CPUS`                  | `8`                              |
| `UV_APPLE_CONTAINER_MEMORY`                | `16g`                            |
| `UV_APPLE_CONTAINER_CARGO_REGISTRY_VOLUME` | `uv-apple-cargo-registry`        |
| `UV_APPLE_CONTAINER_CARGO_GIT_VOLUME`      | `uv-apple-cargo-git`             |
| `UV_APPLE_CONTAINER_TARGET_VOLUME`         | `uv-apple-target`                |
| `UV_APPLE_CONTAINER_UV_CACHE_VOLUME`       | `uv-apple-uv-cache`              |

Trusted runner:

| Variable                | Default                                       |
| ----------------------- | --------------------------------------------- |
| `UV_TRUSTED_IMAGE`      | `ghcr.io/astral-sh/uv:python3.12-trixie-slim` |
| `UV_TRUSTED_DNS`        | `1.1.1.1`                                     |
| `UV_TRUSTED_CPUS`       | `4`                                           |
| `UV_TRUSTED_MEMORY`     | `8g`                                          |
| `UV_TRUSTED_UID_GID`    | unset                                         |
| `UV_TRUSTED_INPUT_DIR`  | unset                                         |
| `UV_TRUSTED_OUTPUT_DIR` | unset                                         |

Untrusted runner:

| Variable                       | Default                                                           |
| ------------------------------ | ----------------------------------------------------------------- |
| `UV_UNTRUSTED_IMAGE`           | `ghcr.io/astral-sh/uv:python3.12-trixie-slim`                     |
| `UV_UNTRUSTED_NETWORK`         | `uv-apple-untrusted`                                              |
| `UV_UNTRUSTED_ALLOWED_DOMAINS` | `pypi.org,files.pythonhosted.org,releases.astral.sh`              |
| `UV_UNTRUSTED_DENIED_DOMAINS`  | `openai.com,*.openai.com,oaiusercontent.com,*.oaiusercontent.com` |
| `UV_UNTRUSTED_ALLOWED_PORTS`   | `80,443`                                                          |
| `UV_UNTRUSTED_CPUS`            | `4`                                                               |
| `UV_UNTRUSTED_MEMORY`          | `8g`                                                              |
| `UV_UNTRUSTED_UID_GID`         | `65532`                                                           |
| `UV_UNTRUSTED_INPUT_DIR`       | unset                                                             |
| `UV_UNTRUSTED_OUTPUT_DIR`      | unset                                                             |
| `UV_UNTRUSTED_SHOW_PROXY_LOG`  | `0`                                                               |

## Compatibility Wrappers

The previous script names still exist as wrappers:

```text
scripts/apple-container-run.sh
scripts/apple-container-untrusted-run.sh
scripts/apple-container-egress-proxy.py
```

Prefer the scripts in this directory for new usage.
