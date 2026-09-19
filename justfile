# phodopus justfile — quality gates and development tasks

default: check

check: fmt-check typecheck clippy test

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

typecheck:
    cargo check --workspace --all-targets

clippy:
    cargo clippy --workspace --all-targets

test:
    cargo test --workspace --all-targets

clean:
    cargo clean
