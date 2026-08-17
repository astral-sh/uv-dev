# The digest pins Rust and its installed component manifest. Match `rust-toolchain.toml`.
FROM --platform=$BUILDPLATFORM rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS build

ENV HOME="/root"
# Fail instead of downloading a different toolchain if the version pin drifts.
ENV RUSTUP_AUTO_INSTALL=0
WORKDIR $HOME

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

# `target add` verifies the archive against the manifest already pinned by the image.
# Do not run `toolchain install`, which would fetch a new manifest.
COPY rust-toolchain.toml rust-toolchain.toml
RUN rustup target add "$(cat rust_target.txt)"

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
