FROM rust:1.83.0-alpine3.20 AS builder

RUN set -ex \
        \
    && apk update \
    && apk upgrade \
    && apk add --update --no-cache musl-dev openssl-dev perl make lld \
    && rustup target add x86_64-unknown-linux-musl

WORKDIR /opt/app

COPY Cargo.toml /opt/app/Cargo.toml

RUN mkdir -p /opt/app/src && echo "fn main() {}" > /opt/app/src/main.rs

RUN --mount=type=cache,target=/usr/local/cargo/registry true \
    set -ex \
        \
    && cargo build --release --target=x86_64-unknown-linux-musl

RUN rm -f /opt/app/src/main.rs
COPY src/ /opt/app/src/

RUN set -ex \
        \
    && export RUSTFLAGS="-C linker=lld" \
    && cargo build --release --target=x86_64-unknown-linux-musl


FROM scratch AS runtime

COPY --from=builder /opt/app/target/x86_64-unknown-linux-musl/release/motdyn /usr/local/bin/motdyn

ENTRYPOINT ["/usr/local/bin/motdyn"]
