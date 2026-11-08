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
cargo rustdoc -- -D warnings
```

## Running tests

```sh
cargo test                          # default features (display-types)
cargo test --no-default-features    # no dependencies; the From impls are left out
```

The README's example runs as a doctest with the others.

## `no_std` builds

The crate is `no_std` without `alloc` in every build. CI checks it on a bare target:

```sh
rustup target add thumbv7em-none-eabi
cargo build --target thumbv7em-none-eabi
cargo build --no-default-features --target thumbv7em-none-eabi
```

## Checking the MSRV

The MSRV is `rust-version` in `Cargo.toml`. CI checks it with
[`cargo-msrv`](https://github.com/foresterre/cargo-msrv), which runs the checks on that
toolchain:

```sh
cargo install cargo-msrv --locked
cargo msrv verify -- cargo test --locked
cargo msrv verify -- cargo test --locked --no-default-features
cargo msrv verify --target thumbv7em-none-eabi \
  -- cargo build --locked --no-default-features --target thumbv7em-none-eabi
```

## Measuring coverage

Coverage requires [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov):

```sh
cargo install cargo-llvm-cov
cargo llvm-cov
```

The current baseline is stored in `.coverage-baseline`. CI fails if coverage drops more
than 0.1% below it. On pushes to `main` or `develop`, an improvement automatically opens
a `ci/coverage-ratchet` PR to commit the new baseline.
