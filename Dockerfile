# syntax=docker/dockerfile:1.7

FROM --platform=$BUILDPLATFORM rust:1.95.0-slim-trixie AS builder

ARG TARGETARCH
ARG CARGO_FEATURE_FLAGS=""

RUN set -eux \
    && case "${TARGETARCH}" in \
        amd64) \
          rust_target="x86_64-unknown-linux-musl"; \
          apt_packages="ca-certificates gcc make musl-tools perl pkg-config"; \
          ;; \
        arm64) \
          rust_target="aarch64-unknown-linux-musl"; \
          dpkg --add-architecture arm64; \
          apt_packages="ca-certificates gcc-aarch64-linux-gnu make musl-dev:arm64 perl pkg-config"; \
          ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
      esac \
    && apt-get update \
    && apt-get install --yes --no-install-recommends ${apt_packages} \
    && rm -rf /var/lib/apt/lists/* \
    && echo "${rust_target}" > /tmp/rust-target \
    && rustup target add "${rust_target}"

ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
    CC_x86_64_unknown_linux_musl=musl-gcc \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc \
    CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc \
    AR_aarch64_unknown_linux_musl=aarch64-linux-gnu-ar \
    RANLIB_aarch64_unknown_linux_musl=aarch64-linux-gnu-ranlib

WORKDIR /opt/app

COPY Cargo.toml Cargo.lock /opt/app/

RUN mkdir -p /opt/app/src && echo "fn main() {}" > /opt/app/src/main.rs

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/opt/app/target \
    set -eux \
    && rust_target="$(cat /tmp/rust-target)" \
    && cargo build --release --locked --target="${rust_target}" ${CARGO_FEATURE_FLAGS}

RUN rm -f /opt/app/src/main.rs
COPY src/ /opt/app/src/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/opt/app/target \
    set -eux \
    && rust_target="$(cat /tmp/rust-target)" \
    && cargo clean --release --target="${rust_target}" -p motdyn \
    && cargo build --release --locked --target="${rust_target}" ${CARGO_FEATURE_FLAGS} \
    && cp "target/${rust_target}/release/motdyn" /tmp/motdyn


FROM scratch AS runtime

COPY --from=builder /tmp/motdyn /usr/local/bin/motdyn

ENTRYPOINT ["/usr/local/bin/motdyn"]
