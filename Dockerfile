FROM --platform=$BUILDPLATFORM ubuntu:24.04@sha256:d1e2e92c075e5ca139d51a140fff46f84315c0fdce203eab2807c7e495eff4f9 AS build

ARG UBUNTU_SNAPSHOT=20260301T000000Z

# Release assets are mutable, so new versions require reviewed SHA-256 digests.
ARG RUSTUP_VERSION=1.28.1
ARG RUSTUP_CHECKSUM_AARCH64=c64b33db2c6b9385817ec0e49a84bcfe018ed6e328fe755c3c809580cc70ce7a
ARG RUSTUP_CHECKSUM_X86_64=a3339fb004c3d0bb9862ba0bce001861fe5cbde9c10d16591eb3f39ee6cd3e7f

# Match the version in `rust-toolchain.toml` and review the manifest hash on each bump.
ARG RUST_VERSION=1.97.1
ARG RUST_TOOLCHAIN_MANIFEST_CHECKSUM=03569b1886ceb5c05276b50c8431ab111de944cd6140fe1fa7d821dd8e0f29cf

ENV HOME="/root"
WORKDIR $HOME

# Retry apt downloads to handle transient mirror failures (e.g., 503s from snapshot.ubuntu.com).
RUN echo 'Acquire::Retries "3";' > /etc/apt/apt.conf.d/80-retries

# Install dependencies using an Ubuntu snapshot for reproducibility.
# ca-certificates are required for using the snapshot.
RUN --mount=type=cache,target=/var/lib/apt/lists \
  apt install -y --update ca-certificates && \
  apt install -y --update --snapshot ${UBUNTU_SNAPSHOT} --no-install-recommends \
  build-essential \
  curl

# Install uv
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv

# Setup zig as cross compiling linker
COPY pyproject.toml uv.lock ./
RUN uv sync --only-group docker --locked
ENV PATH="$HOME/.venv/bin:$PATH"

# Select the cross-compilation target.
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
  "linux/arm64") echo "aarch64-unknown-linux-musl" > rust_target.txt ;; \
  "linux/amd64") echo "x86_64-unknown-linux-musl" > rust_target.txt ;; \
  *) exit 1 ;; \
  esac

# Verify Rustup before running it, without installing a toolchain yet.
RUN arch="$(uname -m)" && \
  case "${arch}" in \
  aarch64) checksum="${RUSTUP_CHECKSUM_AARCH64}" ;; \
  x86_64) checksum="${RUSTUP_CHECKSUM_X86_64}" ;; \
  *) echo "Unsupported rustup host architecture: ${arch}" >&2; exit 1 ;; \
  esac && \
  curl --proto '=https' --tlsv1.2 -sSf \
  "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${arch}-unknown-linux-gnu/rustup-init" \
  -o rustup-init \
  && printf '%s  %s\n' "$checksum" rustup-init | sha256sum -c - \
  && chmod +x rustup-init \
  && ./rustup-init -y --profile minimal --default-toolchain none \
  && rm rustup-init
ENV PATH="$HOME/.cargo/bin:$PATH"

# Rustup normally fetches both the manifest and its checksum from the same server.
# A local mirror makes it verify every component against our pinned manifest.
# Keep later requests local too, so an unpinned toolchain cannot be installed.
COPY rust-toolchain.toml rust-toolchain.toml
ENV RUSTUP_DIST_SERVER="file://$HOME/rust-dist"
# Mirror the minimal Linux profile and the additional musl target. Installing the
# active toolchain also checks that `rust-toolchain.toml` selects the pinned version.
RUN rust_host="$(uname -m)-unknown-linux-gnu" && \
  rust_target="$(cat rust_target.txt)" && \
  manifest="channel-rust-${RUST_VERSION}.toml" && \
  mkdir -p rust-dist/dist && \
  (cd rust-dist/dist && \
    curl --proto '=https' --tlsv1.2 -LsSf \
      "https://static.rust-lang.org/dist/${manifest}" -o "${manifest}" && \
    printf '%s  %s\n' "${RUST_TOOLCHAIN_MANIFEST_CHECKSUM}" "${manifest}" > "${manifest}.sha256" && \
    sha256sum -c "${manifest}.sha256" && \
    release_date="$(sed -n 's/^date = "\(.*\)"$/\1/p' "${manifest}")" && \
    mkdir "${release_date}" && \
    for archive in \
      "rustc-${RUST_VERSION}-${rust_host}.tar.xz" \
      "cargo-${RUST_VERSION}-${rust_host}.tar.xz" \
      "rust-std-${RUST_VERSION}-${rust_host}.tar.xz" \
      "rust-std-${RUST_VERSION}-${rust_target}.tar.xz"; do \
      curl --proto '=https' --tlsv1.2 -LsSf \
        "https://static.rust-lang.org/dist/${release_date}/${archive}" \
        -o "${release_date}/${archive}" || exit 1; \
    done) && \
  rustup toolchain install --profile minimal --no-self-update && \
  rustup target add "${rust_target}" && \
  rm -r rust-dist

# Build
# Build the AWS-LC version pinned in Cargo.lock, not a system installation.
ENV AWS_LC_SYS_USE_SYSTEM=0
COPY crates crates
COPY ./Cargo.toml Cargo.toml
COPY ./Cargo.lock Cargo.lock

# Install cargo-auditable
RUN cargo install \
  --locked \
  --version 0.7.4 \
  cargo-auditable

RUN case "${TARGETPLATFORM}" in \
  "linux/arm64") export JEMALLOC_SYS_WITH_LG_PAGE=16;; \
  esac && \
  cargo auditable zigbuild --bin uv --bin uvx --target $(cat rust_target.txt) --release
RUN cp target/$(cat rust_target.txt)/release/uv /uv \
  && cp target/$(cat rust_target.txt)/release/uvx /uvx
# TODO(konsti): Optimize binary size, with a version that also works when cross compiling
# RUN strip --strip-all /uv

FROM scratch
COPY --from=build /uv /uvx /
WORKDIR /io
ENTRYPOINT ["/uv"]
