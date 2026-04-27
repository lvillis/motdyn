# syntax=docker/dockerfile:1.7

FROM --platform=$TARGETPLATFORM rust:1.95.0-alpine3.23 AS builder

ARG TARGETARCH
ARG CARGO_FEATURE_FLAGS=""

RUN set -eux \
    && case "${TARGETARCH}" in \
        amd64) rust_target="x86_64-unknown-linux-musl" ;; \
        arm64) rust_target="aarch64-unknown-linux-musl" ;; \
        *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
      esac \
    && echo "${rust_target}" > /tmp/rust-target \
    && apk add --no-cache musl-dev openssl-dev perl make lld \
    && rustup target add "${rust_target}"

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
    && export RUSTFLAGS="-C linker=lld" \
    && cargo build --release --locked --target="${rust_target}" ${CARGO_FEATURE_FLAGS} \
    && cp "target/${rust_target}/release/motdyn" /tmp/motdyn


FROM scratch AS runtime

COPY --from=builder /tmp/motdyn /usr/local/bin/motdyn

ENTRYPOINT ["/usr/local/bin/motdyn"]
