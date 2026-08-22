set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

format:
    cargo fmt --all

format-check:
    cargo fmt --all -- --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

cargo-check:
    cargo check --workspace --locked

build:
    cargo build --workspace --locked

test:
    cargo test --workspace --locked

test-integration:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -d tests ]]; then
        cargo test --workspace --locked --test '*'
    else
        printf '%s\n' "No integration test targets."
    fi

deny:
    cargo deny check

machete:
    cargo machete

check: format-check clippy cargo-check build deny machete

verify: check test
