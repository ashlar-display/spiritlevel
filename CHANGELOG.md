# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **docs.rs marks the items that need the `display-types` feature** (the `From` impls for
  display-types' sink capability types) with the `doc_cfg` badge, enabled only in docs.rs
  builds.

### Changed

- **The README shows the "trusted publishing only" badge**: crates.io refuses token
  uploads for this crate, and "Verifying releases" says so.

### Internal

- Releases publish through crates.io trusted publishing only: the first-publish token path
  in publish.yml and the first-release path in release-prep, used once for 0.1.0, are
  removed.
- **CI's test and MSRV jobs do not keep the checkout's git credentials**
  (`persist-credentials: false`); they never push. The coverage job keeps them to push its
  ratchet branch.
- **CI fails on a `[patch]` section in `Cargo.toml`**, on `main` and in the publish
  workflow's `verify` job: `cargo package` drops it, so a release built with one would not
  build from crates.io.
- **rustdoc runs with every feature**, as docs.rs builds, in CI and in the publish
  workflow's `verify` job.
- **The MSRV job shows why it failed**: `cargo msrv verify` runs without
  `--output-format minimal`, which printed only `true` or `false` and dropped the compiler
  error.
- **The coverage job's cache is keyed on the Rust version** as well as `Cargo.lock`, so a
  `target/` built by an older Rust is not restored: `cargo llvm-cov clean --workspace`
  leaves its instrumented binaries behind, and the report would count their lines as
  uncovered (it did so in other Ashlar crates after the update to Rust 1.100). The
  coverage run is `--locked`, as the other jobs are.
- **The documentation is built as docs.rs builds it, weekly**: `docs.yml` runs
  `cargo doc --all-features` with nightly and `--cfg docsrs` (and `-D warnings`), so a
  nightly change to the `doc_cfg` feature badges shows up before it breaks a docs.rs build.
- **`doc/testing.md` describes the tests**: what each module's unit tests check, the README
  doctest, the builds CI runs and the coverage ratchet.
- **The docs give the coverage tolerance as 0.1 percentage points everywhere** (as CI
  checks it, not 0.1%), `doc/setup.md` installs the cargo-msrv version CI pins (0.19.6),
  `CONTRIBUTING.md` has a publishing section, and the README links `doc/testing.md`.
- **`doc/setup.md` says to measure the coverage baseline with the Rust version CI uses**:
  Rust versions count lines differently, so a baseline measured with another version can
  fail CI without a change in coverage.

## [0.1.0] - 2026-11-08

### Added

**Link parameters**

- `TmdsLinkParams::choose` — a pure, I/O-free `const fn` that chooses a TMDS link from the
  character rate (kHz), the sink's and source's facts and the caller's policy: the bit
  clock ratio (`ClockRatio`), whether the link is scrambled, and what the sink's
  `TMDS_Config` must hold (`SinkTmdsConfig`, `None` for a sink without SCDC).
- `ScramblingDecision` — why the link is or is not scrambled: `Required` above the
  threshold, `LowRate` at or below it, or `Off` with a `ScramblingOff` carrying every
  condition that applies (no SCDC, no scrambling support, the sink does not declare
  `LTE_340Mcsc_scramble`, the source cannot, the policy is off).
- `ParamsError::ScramblingUnsupported` — a rate above 340 000 kHz on a sink without SCDC
  or scrambling support, naming every missing fact.

**Inputs**

- `SinkTmdsFacts` (`new`, `NONE`), `SourceTmdsFacts` (`new`).
- `TmdsPolicy` — low-rate scrambling (default on) and `ScramblingThreshold` (default
  `Above340`, or `AtOrAbove340`), with `DEFAULT` and `const` builders.

**Features**

- `display-types` (default) — `From<&HdmiForumSinkCap>` and
  `From<Option<&HdmiForumSinkCap>>` for `SinkTmdsFacts`. Without it the crate has no
  dependencies.

### Internal

- CI: format, clippy and docs with `-D warnings`, tests with and without default
  features, and `no_std` builds for `thumbv7em-none-eabi` with and without them, with a
  read-only token except where the coverage job opens its PR.
- MSRV check: `cargo msrv verify` runs the tests (with and without default features) and
  the bare `no_std` build on the `rust-version` in `Cargo.toml` (1.85).
- The README's example is compiled and run as a doctest, so it cannot fall behind the API.
- Coverage ratchet: CI measures line coverage against `.coverage-baseline` and opens a PR
  when it rises.
- Weekly RustSec audit, and Dependabot for the workflow pins and `Cargo.lock`.
- Release workflows (release-prep, release-tag, publish) as in the other crates, with a
  first-release path: release-prep releases the version in `Cargo.toml` when the crate is
  not on crates.io yet, and publish uses a first-publish token where trusted publishing
  is not yet possible. release-prep refuses a version that is already tagged or already
  has a CHANGELOG section. publish's verify and MSRV jobs mirror CI.
