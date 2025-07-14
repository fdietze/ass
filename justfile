check:
  cargo clippy --all-targets
  cargo test --workspace --all-targets

fix:
  cargo clippy --fix --allow-dirty --allow-staged --all-targets
  cargo fmt
