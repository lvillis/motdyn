set shell := ["bash", "-euo", "pipefail", "-c"]

patch:
    cargo release patch --no-publish --execute

publish:
    cargo publish

ci:
    cargo fmt --all
    cargo check --all-features
    cargo check --no-default-features
    cargo clippy --all-targets --all-features -- -D warnings
    cargo clippy --all-targets --no-default-features -- -D warnings
    cargo nextest run --all-features --locked
    cargo nextest run --no-default-features --locked
