# Development Setup

**Requirements:** Rust 1.85+ (stable). Install via [rustup](https://rustup.rs/).

## Clone and build

```sh
git clone https://github.com/ashlar-display/spiritlevel.git
cd spiritlevel
cargo build
```

## Running checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy --all-targets --features culvert -- -D warnings
cargo rustdoc --all-features -- -D warnings
```

## Running tests

```sh
cargo test                          # default features (display-types)
cargo test --no-default-features    # without display-types' From impls
cargo test --features culvert       # with the adapter for culvert's SCDC client
```

The README's examples run as doctests with the others.

## `no_std` builds

The crate is `no_std` without `alloc` in every build. CI checks it on a bare target:

```sh
rustup target add thumbv7em-none-eabi
cargo build --target thumbv7em-none-eabi
cargo build --no-default-features --target thumbv7em-none-eabi
cargo build --features culvert --target thumbv7em-none-eabi
```

## Checking the MSRV

The MSRV is `rust-version` in `Cargo.toml`. CI checks it with
[`cargo-msrv`](https://github.com/foresterre/cargo-msrv), which runs the checks on that
toolchain:

```sh
cargo install cargo-msrv --locked
cargo msrv verify -- cargo test --locked
cargo msrv verify -- cargo test --locked --no-default-features
cargo msrv verify -- cargo test --locked --features culvert
cargo msrv verify --target thumbv7em-none-eabi \
  -- cargo build --locked --no-default-features --target thumbv7em-none-eabi
```

## Measuring coverage

Coverage requires [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```sh
cargo install cargo-llvm-cov
cargo llvm-cov clean --workspace
cargo llvm-cov --no-report
cargo llvm-cov --no-report --features culvert
cargo llvm-cov report
```

The second run covers the `culvert` adapter; the report merges both, as CI does.

The current baseline is stored in `.coverage-baseline`. CI fails if coverage drops more
than 0.1% below it. On pushes to `main` or `develop`, an improvement automatically opens
a `ci/coverage-ratchet` PR to commit the new baseline.
