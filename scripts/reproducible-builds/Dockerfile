# Keep the Rust image and target archive in sync with rust-toolchain.toml.
FROM docker.io/library/rust:1.97.1-bookworm@sha256:408fe88047cef61a2087653b0c5255fa51c0f2d6d94ddedd7a2562a9b91a46f6 AS rust

# This is the same cross-compiler image family used by maturin-action for musl.
# Pin the amd64 image, including its native compiler, sysroot, and build tools.
FROM ghcr.io/rust-cross/rust-musl-cross:x86_64-musl@sha256:fb7229c2a7dd84957e3a46f8bec7d01c4ec7c295ad159606623093a0a9286253

COPY --from=rust /usr/local/rustup /usr/local/rustup
COPY --from=rust /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup \
    RUSTUP_TOOLCHAIN=1.97.1 \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/usr/local/musl/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-unknown-linux-musl-gcc \
    AWS_LC_SYS_USE_SYSTEM=0

WORKDIR /build-tools
RUN curl --proto '=https' --tlsv1.2 -fsSL \
      https://static.rust-lang.org/dist/2026-07-16/rust-std-1.97.1-x86_64-unknown-linux-musl.tar.xz \
      -o rust-std.tar.xz \
    && echo '51d83178680556f73a5fa8ad865b76a1ff541867445c00fc65dc67246bc2de66  rust-std.tar.xz' | sha256sum -c - \
    && tar -xf rust-std.tar.xz \
    && ./rust-std-1.97.1-x86_64-unknown-linux-musl/install.sh \
      --prefix="$(rustc --print sysroot)" --disable-ldconfig \
    && rm -rf rust-std.tar.xz rust-std-1.97.1-x86_64-unknown-linux-musl

# Match scripts/install-cargo-extensions.sh, but install for the GNU host.
RUN cargo install --locked --version 0.7.4 \
    --target x86_64-unknown-linux-gnu cargo-auditable

COPY build.sh /usr/local/bin/build-reproducible-release
ENTRYPOINT ["bash", "/usr/local/bin/build-reproducible-release"]
