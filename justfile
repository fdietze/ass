# https://github.com/casey/just

# List available recipes in the order in which they appear in this file
_default:
  @just --list --unsorted

check:
  cargo clippy --all-targets
  cargo test --workspace --all-targets

fix:
  cargo clippy --fix --allow-dirty --allow-staged --all-targets
  cargo fmt
