FROM --platform=$BUILDPLATFORM ubuntu:24.04@sha256:d1e2e92c075e5ca139d51a140fff46f84315c0fdce203eab2807c7e495eff4f9 AS build

ARG UBUNTU_SNAPSHOT=20260301T000000Z

# Match `rust-toolchain.toml` and review the archive hashes on each version bump.
ARG RUST_VERSION=1.97.1
ARG RUST_CHECKSUM_AARCH64=9a7a2c336b4787f1b72f6bab7c35d5b7af2fd03cbd39b4fc721466a70d402a7d
ARG RUST_CHECKSUM_X86_64=88f28fa9af20594179f85d6df67078dfd6fa93e2f6da5e1e9b0ac4997988ca4f
ARG RUST_STD_CHECKSUM_AARCH64=49ff0879d94e2e8e86d5e85eb15a9215943e8c78b51363d6553443598cab5d31
ARG RUST_STD_CHECKSUM_X86_64=51d83178680556f73a5fa8ad865b76a1ff541867445c00fc65dc67246bc2de66

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
  curl \
  xz-utils

# Install uv
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv

# Setup zig as cross compiling linker
COPY pyproject.toml uv.lock ./
RUN uv sync --only-group docker --locked
ENV PATH="$HOME/.venv/bin:$PATH"

# Without Rustup, Cargo does not enforce `rust-toolchain.toml` for us.
COPY rust-toolchain.toml rust-toolchain.toml
RUN grep -Fx "channel = \"${RUST_VERSION}\"" rust-toolchain.toml

# Install only the compiler, Cargo, and host standard library from the verified archive.
RUN rust_host="$(uname -m)-unknown-linux-gnu" && \
  case "${rust_host}" in \
  aarch64-unknown-linux-gnu) checksum="${RUST_CHECKSUM_AARCH64}" ;; \
  x86_64-unknown-linux-gnu) checksum="${RUST_CHECKSUM_X86_64}" ;; \
  *) echo "Unsupported Rust host: ${rust_host}" >&2; exit 1 ;; \
  esac && \
  archive="rust-${RUST_VERSION}-${rust_host}" && \
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://static.rust-lang.org/dist/${archive}.tar.xz" -o "${archive}.tar.xz" && \
  printf '%s  %s\n' "${checksum}" "${archive}.tar.xz" | sha256sum -c - && \
  tar -xJf "${archive}.tar.xz" && \
  "./${archive}/install.sh" --prefix=/usr/local \
    --components="rustc,cargo,rust-std-${rust_host}" --disable-ldconfig && \
  rm -r "${archive}" "${archive}.tar.xz"

# Install the independently pinned musl standard library into the same toolchain.
ARG TARGETPLATFORM
RUN case "${TARGETPLATFORM}" in \
  "linux/arm64") rust_target="aarch64-unknown-linux-musl"; checksum="${RUST_STD_CHECKSUM_AARCH64}" ;; \
  "linux/amd64") rust_target="x86_64-unknown-linux-musl"; checksum="${RUST_STD_CHECKSUM_X86_64}" ;; \
  *) echo "Unsupported Rust target: ${TARGETPLATFORM}" >&2; exit 1 ;; \
  esac && \
  printf '%s\n' "${rust_target}" > rust_target.txt && \
  archive="rust-std-${RUST_VERSION}-${rust_target}" && \
  curl --proto '=https' --tlsv1.2 -LsSf \
    "https://static.rust-lang.org/dist/${archive}.tar.xz" -o "${archive}.tar.xz" && \
  printf '%s  %s\n' "${checksum}" "${archive}.tar.xz" | sha256sum -c - && \
  tar -xJf "${archive}.tar.xz" && \
  "./${archive}/install.sh" --prefix=/usr/local --disable-ldconfig && \
  rm -r "${archive}" "${archive}.tar.xz"
ENV PATH="$HOME/.cargo/bin:$PATH"

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
