# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking changes

- **`ClockRatio` is hdmi-hal's `TmdsClockRatio`**, re-exported under the old name, so the
  link parameters pass straight to the PHY. The variants (`Div10`, `Div40`) and derives are
  the same; what changes is that spiritlevel now depends on hdmi-hal, and with it always on
  display-types.

### Added

- **`SourceTmdsFacts::scrambling`**: whether the source has a TMDS scrambler, with
  `with_scrambling` and `From<&TmdsPhyCapabilities>`, which fills it and
  `low_rate_scrambling` from the PHY's capabilities report. `choose` treats a source without
  a scrambler as it treats a sink that cannot take scrambling: refused above 340 000 kHz
  (`ScramblingUnsupported` gains `source_cannot_scramble`), the plain 1/10 link at exactly
  340 000 kHz under `AtOrAbove340` and at or below the threshold (`ScramblingOff::source_cannot`).
  `new` still takes the source to have a scrambler, so 0.1's results do not change.
- **`SourceTmdsFacts::max_character_rate_khz`**: the source's own highest TMDS character
  rate, with `with_max_character_rate_khz` and filled by `From<&TmdsPhyCapabilities>`.
  `choose` checks it first and refuses a rate above it with the new
  `ParamsError::AboveSourceMaximum { rate_khz, max_khz }`, so a link the source cannot drive
  never gets a sink configuration. `new` sets `u32::MAX`, so 0.1's results do not change.
- **The SCDC vocabulary the procedures will use.** `SinkTmdsConfig::from_read` builds a
  value an SCDC implementation decoded from a byte, which `raw()` keeps (`None` for one the
  caller built); equality and hashing compare the two fields only, so a value read back
  equals the one written. `ScramblingStatus` is what the sink reports in
  `Scrambling_Status`, with the byte read (`raw()`). `UpdateError<E>` (`Read` or `Write`) is
  the error of an operation made of a read and a write, saying which failed.

### Changed

- **The README shows the "trusted publishing only" badge**: crates.io refuses token
  uploads for this crate, and "Verifying releases" says so.

### Internal

- CI and the publish workflow's verify job lint and build the `culvert` feature, for the
  host and the bare `thumbv7em-none-eabi` target.
- Releases publish through crates.io trusted publishing only: the first-publish token path
  in publish.yml and the first-release path in release-prep, used once for 0.1.0, are
  removed.

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
