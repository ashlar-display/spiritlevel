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
- **`TmdsScdc`**, the sink-side SCDC operations the procedures make: `TMDS_Config` written
  whole (`write_tmds_config`) or read-modify-written (`update_tmds_config`), read back
  (`read_tmds_config`), `Scrambling_Status`, `Sink_Version` and `Source_Version`, and the FRL
  clear (`clear_frl_rate`, `clear_flt_update`). Its error is a `DdcError`, so a procedure
  can tell a failure that will pass from one that will not. A trait of the crate's own, so
  an SCDC implementation without culvert can provide it.
- **The `culvert` feature: `TmdsScdc` for culvert's client** (`culvert::Scdc<T>`, over any
  `ScdcTransport`), with culvert's errors wrapped in `CulvertError`, whose `DdcError` kind
  is the transport's own, or `Unsupported` for a request culvert refuses. `update_tmds_config`
  keeps the reserved bits through culvert's own read-modify-write, and the raw bytes of
  `TMDS_Config` and `Scrambling_Status` reach `SinkTmdsConfig::raw` and
  `ScramblingStatus::raw`.
- **The procedures' settings**, ahead of the procedures: `BringUpConfig` (the ratio wait,
  the scrambling deadline, the check's window and interval, and the settings
  `ClearFrl`, `SourceVersionWrite`, `OutputDuringSinkWrite`, `TmdsConfigWrite`,
  `OnTmdsConfigFailure`, `tear_down_on_error` and its `TearDownConfig`), `TearDownConfig`
  (`clear_sink`, `SinkClearTiming`, `ScramblerOff`) and `SinkPresence`. Each has a
  `DEFAULT` and `const fn` setters.
- **Diagnostics, starting with tear-down.** `TmdsEvent` (a `Timestamp` and a
  `TmdsEventKind`), `Copy` and without type parameters: a failed step is recorded by its
  `TmdsStep` and, for SCDC, its `DdcErrorKind`, never by the error value. Tear-down's kinds
  (started, `TMDS_Config` written, sink clear skipped with its `SinkClearSkip`, scrambler
  set, output stopped, step failed, ended) and its bound, `MAX_TEAR_DOWN_EVENTS`. Its
  results: `TornDown`, `TearDownError` (every step attempted, the first failure of each kept)
  and `SinkClear`.
- **`procedure::TmdsIo` and `procedure::tear_down`**, the first procedure of the I/O-free
  core: `TmdsIo` flattens the SCDC, PHY, clock, delay and cancel flag into one trait the
  procedures are written over once, as `async fn`. Tear-down stops the output; with
  `clear_sink` it also writes the sink's `TMDS_Config` to 0 and turns the scrambler off, in
  the order `SinkClearTiming` and `ScramblerOff` give, writing nothing to a plain link, a
  sink without SCDC or a sink the caller says is gone. Every step is attempted: a failed
  sink write is a warning, a failed PHY call still lets the output stop.
- **`TmdsLink`, the sync driver**, over a `TmdsScdc`, a `TmdsPhy`, a `Clock` and a `Delay`
  (one value can serve as both through a reference), with `tear_down` and
  `tear_down_with_events`: the procedure runs to completion before the call returns, with
  no runtime. A cancel flag through the `Cancel` trait, implemented for `&AtomicBool`, with
  `NoCancel` the default (`with_cancel`); `into_parts` gives the parts back.
- **The reconnect check**, `procedure::check_reconnect` and `TmdsLink::check_reconnect`
  (with its `_with_events` form): after a hotplug while the link is up, one read of the
  sink's `TMDS_Config`, compared with what the link needs on both bits, a plain link
  included. `ReconnectCheck` is `Matches`, `Lost` (with the bits and byte read),
  `Unreadable`, `NotApplicable` (no SCDC) or `SinkChanged`: the caller's current sink facts
  differ from those the link was chosen with, and nothing is read. It writes nothing.
  `TmdsLinkParams::sink()` gives the sink facts a link was chosen from, and
  `MAX_RECONNECT_CHECK_EVENTS` the check's bound.
- **Bring-up's result types**, ahead of the procedure: `BringUpOutcome` (`Up` with a
  `LinkUp`, or `Cancelled` with the tear-down's result), `BringUpError` (`Scdc` or `Phy`,
  with the step, the output's state, the `Cleanup` and the warnings gathered), the six
  `BringUpWarning` kinds in fixed `Warnings` storage, `ScramblingCheck` with `CheckResult`
  and `FailedRead`, `OutputState` and `WriteStage`. `TmdsStep` gains the bring-up steps, so
  an error and its event name a step alike.

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
