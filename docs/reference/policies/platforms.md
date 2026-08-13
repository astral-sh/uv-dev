# Platform support

uv has Tier 1 support for the following platforms:

- macOS (Apple Silicon)
- macOS (x86_64)
- Linux (x86_64)
- Windows (x86_64)

uv is continuously built and tested on Tier 1 platforms. Following the Rust project, Tier 1 means
["guaranteed to work"](https://doc.rust-lang.org/beta/rustc/platform-support.html#tier-1).

uv has Tier 2 support
(["guaranteed to build"](https://doc.rust-lang.org/beta/rustc/platform-support.html#tier-2-with-host-tools))
for the following platforms:

- Linux (PPC64LE)
- Linux (RISC-V64)
- Linux (aarch64)
- Linux (armv7)
- Linux (i686)
- Linux (s390x)
- Windows (arm64)

uv has Tier 3 support
(["best effort"](https://doc.rust-lang.org/beta/rustc/platform-support.html#tier-3)) for the
following platforms:

- FreeBSD (x86_64)
- Windows (i686)

uv provides official binaries on GitHub and pre-built wheels on [PyPI](https://pypi.org/project/uv/)
for its Tier 1 and Tier 2 platforms.

uv is continuously built for Tier 2 platforms, but the test suite does not run on them. Stability
may vary.

Tier 3 platforms may not receive regular builds or tests. uv accepts patches that fix bugs on these
platforms.

## Linux versions

On Linux, the libc version determines compatibility.

uv publishes both glibc-based and musl-based distributions.

For glibc-based Linux distributions, uv publishes
[manylinux-compatible](https://peps.python.org/pep-0600/) wheels and corresponding binaries. These
artifacts require glibc on the host system. A manylinux wheel tag includes the minimum supported
glibc version. For example, `manylinux_2_17_x86_64` requires glibc 2.17 or later.

uv publishes official glibc-based wheels and binaries for the following targets:

- `x86_64-unknown-linux-gnu` (`manylinux_2_17_x86_64`)
- `aarch64-unknown-linux-gnu` (`manylinux_2_28_aarch64`)
- `armv7-unknown-linux-gnueabihf` (`manylinux_2_17_armv7l`)
- `i686-unknown-linux-gnu` (`manylinux_2_17_i686`)
- `powerpc64le-unknown-linux-gnu` (`manylinux_2_17_ppc64le`)
- `riscv64gc-unknown-linux-gnu` (`manylinux_2_31_riscv64`)
- `s390x-unknown-linux-gnu` (`manylinux_2_17_s390x`)

uv also publishes musl-based wheels and fully statically linked binaries for the following targets:

- `x86_64-unknown-linux-musl` (`musllinux_1_1_x86_64`)
- `aarch64-unknown-linux-musl` (`musllinux_1_1_aarch64`)
- `armv7-unknown-linux-musleabihf` (`musllinux_1_1_armv7l`)
- `i686-unknown-linux-musl` (`musllinux_1_1_i686`)
- `riscv64gc-unknown-linux-musl` (`musllinux_1_1_riscv64`)
- `arm-unknown-linux-musleabihf` (`linux_armv6l`)

These wheels have [musllinux-compatible](https://peps.python.org/pep-0656/) tags. The included `uv`
binaries are fully statically linked, so the host system does not need musl libc.

The official [Docker images](../../guides/integration/docker.md) include these fully statically
linked musl uv binaries for amd64 and arm64.

## Windows versions

The minimum supported Windows versions are Windows 10 and Windows Server 2016. These match
[Rust's own Tier 1 support](https://blog.rust-lang.org/2024/02/26/Windows-7.html).

## macOS versions

uv supports macOS 13+ (Ventura).

uv also works on macOS 12 if a `realpath` executable is installed.
