FROM --platform=$BUILDPLATFORM ubuntu:24.04@sha256:d1e2e92c075e5ca139d51a140fff46f84315c0fdce203eab2807c7e495eff4f9 AS build

ARG UBUNTU_SNAPSHOT=20260301T000000Z
# Release assets are mutable, so new versions require reviewed SHA-256 digests.
ARG RUSTUP_VERSION=1.28.1
ARG RUSTUP_CHECKSUM_AARCH64=c64b33db2c6b9385817ec0e49a84bcfe018ed6e328fe755c3c809580cc70ce7a
ARG RUSTUP_CHECKSUM_X86_64=a3339fb004c3d0bb9862ba0bce001861fe5cbde9c10d16591eb3f39ee6cd3e7f
# Keep this checksum in sync with rust-toolchain.toml.
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

# Install rust
ARG TARGETPLATFORM
RUN case "$TARGETPLATFORM" in \
  "linux/arm64") echo "aarch64-unknown-linux-musl" > rust_target.txt ;; \
  "linux/amd64") echo "x86_64-unknown-linux-musl" > rust_target.txt ;; \
  *) exit 1 ;; \
  esac

RUN arch="$(uname -m)" && \
  case "${RUSTUP_VERSION}:${arch}" in \
  1.28.1:aarch64) checksum="${RUSTUP_CHECKSUM_AARCH64}" ;; \
  1.28.1:x86_64) checksum="${RUSTUP_CHECKSUM_X86_64}" ;; \
  *) echo "No trusted rustup checksum for version ${RUSTUP_VERSION} (${arch})" >&2; exit 1 ;; \
  esac && \
  curl --proto '=https' --tlsv1.2 -sSf \
  "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${arch}-unknown-linux-gnu/rustup-init" \
  -o rustup-init \
  && printf '%s  %s\n' "$checksum" rustup-init | sha256sum -c - \
  && chmod +x rustup-init \
  && ./rustup-init -y --target $(cat rust_target.txt) --profile minimal --default-toolchain none \
  && rm rustup-init
ENV PATH="$HOME/.cargo/bin:$PATH"
# Rustup normally fetches both the manifest and its checksum from the same server.
# Use a local mirror so every component is checked against our pinned manifest.
COPY rust-toolchain.toml rust-toolchain.toml
# Keep later Rustup requests local, even after the downloaded archives are removed.
ENV RUSTUP_DIST_SERVER="file://$HOME/rust-dist"
# Fetch the minimal Linux toolchain and musl target. Rustup verifies the archives
# against the local manifest before installing them; disable its self-update too.
RUN rust_version="$(sed -n 's/^channel = "\(.*\)"$/\1/p' rust-toolchain.toml)" && \
  rust_host="$(uname -m)-unknown-linux-gnu" && \
  rust_target="$(cat rust_target.txt)" && \
  manifest="channel-rust-${rust_version}.toml" && \
  mkdir -p rust-dist/dist && \
  curl --proto '=https' --tlsv1.2 -LsSf \
  "https://static.rust-lang.org/dist/${manifest}" -o "rust-dist/dist/${manifest}" && \
  printf '%s  %s\n' "${RUST_TOOLCHAIN_MANIFEST_CHECKSUM}" "${manifest}" > "rust-dist/dist/${manifest}.sha256" && \
  (cd rust-dist/dist && sha256sum -c "${manifest}.sha256") && \
  release_date="$(sed -n 's/^date = "\(.*\)"$/\1/p' "rust-dist/dist/${manifest}")" && \
  mkdir "rust-dist/dist/${release_date}" && \
  for archive in \
  "rustc-${rust_version}-${rust_host}.tar.xz" \
  "cargo-${rust_version}-${rust_host}.tar.xz" \
  "rust-std-${rust_version}-${rust_host}.tar.xz" \
  "rust-std-${rust_version}-${rust_target}.tar.xz"; do \
  curl --proto '=https' --tlsv1.2 -LsSf \
  "https://static.rust-lang.org/dist/${release_date}/${archive}" \
  -o "rust-dist/dist/${release_date}/${archive}" || exit 1; \
  done \
  && rustup toolchain install --profile minimal --no-self-update \
  && rustup target add "${rust_target}" \
  && rm -r rust-dist

# Build
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
