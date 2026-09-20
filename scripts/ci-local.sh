#!/usr/bin/env bash

set -euo pipefail

project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_root"

echo "Checking formatting"
cargo fmt --all --check

echo "Checking the native workspace"
cargo check --workspace --all-targets --locked

echo "Running native lints"
cargo clippy --workspace --all-targets --locked -- -D warnings

echo "Running native tests"
cargo test --workspace --locked

cross_targets=(
    x86_64-pc-windows-msvc
    x86_64-apple-darwin
    aarch64-apple-darwin
)

for target in "${cross_targets[@]}"; do
    if ! rustup target list --installed | rg --fixed-strings --line-regexp "$target" >/dev/null; then
        echo "Missing Rust target: $target" >&2
        echo "Install it with: rustup target add $target" >&2
        exit 1
    fi

    echo "Checking $target"
    cargo check --workspace --all-targets --target "$target" --locked

    echo "Running lints for $target"
    cargo clippy --workspace --all-targets --target "$target" --locked -- -D warnings
done

if [[ "${1:-}" == "--hardware" ]]; then
    echo "Running native camera detection"
    cargo run --quiet
fi

echo "Local CI completed successfully"
