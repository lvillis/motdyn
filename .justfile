set shell := ["bash", "-euo", "pipefail", "-c"]

answer-check:
  cargo fmt --all --check
  cargo check --locked --all-targets
  cargo clippy --locked --all-targets -- -D warnings

verify: answer-check

ci: answer-check
  cargo test --locked
  cargo run --locked -- --help >/dev/null
  cargo run --locked -- status --help >/dev/null

release-check: ci
  cargo build --release --locked

patch:
  cargo release patch --no-publish --execute
