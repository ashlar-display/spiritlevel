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
- **Equality of the sink facts and of the link includes the new fields.** `SinkTmdsFacts`
  compares and hashes `frl_capable` too, so facts converted from an EDID that declares no
  FRL differ from `SinkTmdsFacts::new`'s for a sink with SCDC; `TmdsLinkParams` compares
  and hashes the sink facts, source facts and policy it was chosen from. `NONE == new(false, false, false)` holds as
  in 0.1.

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
- **The SCDC vocabulary of the procedures.** `TmdsConfigReading` is what a read of
  `TMDS_Config` returns: the two fields an SCDC implementation decoded, as a
  `SinkTmdsConfig`, and the byte as read (`raw`), so a sink that sets reserved bits stays
  visible. `ScramblingStatus` is what the sink reports in
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
  `Unreadable`, `NotApplicable` (no SCDC) or `SinkChanged`: the link no longer fits the
  caller's current sink facts, and nothing is read. It writes nothing.
  `TmdsLinkParams::sink()` gives the sink facts a link was chosen from, `fits` whether
  choosing again for other sink facts gives the same ratio, scrambling and `TMDS_Config`
  (a fact the link does not depend on, such as FRL, may differ), and
  `MAX_RECONNECT_CHECK_EVENTS` the check's bound.
- **Bring-up's result types**, ahead of the procedure: `BringUpOutcome` (`Up` with a
  `LinkUp`, or `Cancelled` with where the flag was seen, the `Cleanup` and the warnings
  gathered), `BringUpError` (`Scdc` or `Phy`,
  with the step, the output's state, the `Cleanup` and the warnings gathered), the seven
  `BringUpWarning` kinds in fixed `Warnings` storage, `ScramblingCheck` with `CheckResult`
  and `FailedRead`, and `OutputState`. `TmdsStep` gains the bring-up steps, so an error and
  its event name a step alike; the read of a read-modify-write is `ReadTmdsConfig`, its
  write `WriteTmdsConfig`. A failed read-and-write is an `UpdateError` in every value,
  which is exhaustive and a `DdcError` with the failed half's kind.
- **`SinkTmdsFacts::frl_capable`**: the sink declares FRL, from a non-zero `Max_FRL_Rate`
  in the `From` impls, with `with_frl_capable`. `new` sets it for a sink with SCDC, so a
  caller that never learns of it still gets the FRL clear before a bring-up; `NONE` and a
  block without FRL leave it clear. With it, the events of the work before a bring-up: the FRL clear
  (`FrlRateCleared`, `FltUpdateCleared`, `FrlClearSkipped` with its `FrlClearSkip`) and
  `Source_Version` (`SinkVersionRead`, `SourceVersionWritten`, `SourceVersionSkipped` with its
  `SourceVersionSkip`).
- **`TmdsPolicy::tmds_config_gate`**: which sinks get the `TMDS_Config` write on a plain
  link, `TmdsConfigGate::AnyScdcSink` (the default, 0.1's rule) or `ScramblingSink`, with
  `with_tmds_config_gate`. Under `ScramblingSink`, a plain link on a sink with SCDC but no
  scrambling support gets no write: `sink_config()` is `None` exactly when no write is due,
  and is unchanged under the default.
- **Bring-up**, `procedure::bring_up` and `TmdsLink::bring_up` (with its `_with_events`
  form): the FRL clear and `Source_Version` on a sink with SCDC, then the output stopped, the
  sink's `TMDS_Config` written (read-modify-write or whole byte; skipped where no write is due,
  with `TmdsConfigSkip`), the PHY configured with its scrambler, the ratio wait spent from the
  write, and the output started. With `OutputDuringSinkWrite::Running`, unscrambled output
  runs through the write and the scrambler comes on after the wait. A failed `TMDS_Config`
  write carries on as a warning under `CarryOn`, with the register read back on a plain
  link; a missed deadline is a warning on a scrambled link; a failed FRL clear,
  `Sink_Version` or `Source_Version` is a warning whatever the error's kind. A PHY failure,
  a non-transient error of the `TMDS_Config` write or a failed write under `Stop` ends it
  with `BringUpError`, after a tear-down
  unless `tear_down_on_error` is off (with the sink taken as gone after `Detached`). Its
  events and `MAX_BRING_UP_EVENTS`.
- **The scrambling check in bring-up**: on a scrambled link, `Scrambling_Status` polled
  every `check_interval` until the sink reports scrambling or `check_window` ends, timed
  from the output start (from the scrambler coming on with `Running`). The result is in
  `LinkUp::check`: `Set` with when, `NotSet`, or `Unreadable` when every read failed or one
  failed with `Detached` or `Unsupported`; other failed reads are counted with the first
  kept. A read count of `ceil(window / interval) + 1` ends the check on a clock that does
  not advance, marked `read_cap_reached`. One `ScramblingChecked` event carries it.
- **Cancellation in bring-up**: the cancel flag is read at entry, before each SCDC step,
  before each slice of the ratio wait (spent in slices of at most `check_interval`, so a
  cancel is seen within one interval) and before each read of the check. Once set, the
  bring-up ends `BringUpOutcome::Cancelled`, never an error: at entry before any I/O
  (`Cleanup::Skipped`), otherwise after a tear-down with the caller's `tear_down` settings,
  whatever `tear_down_on_error` says. `CancelPoint` says where it was seen, in the outcome
  and in the new `BringUpCancelled` event; `BringUpEnded` gains `cancelled`. Tear-down and
  the reconnect check are not cancelled, and the flag is never cleared.
- **The quick restore**, `procedure::quick_restore` and `TmdsLink::quick_restore` (with its
  `_with_events` form): the option to a tear-down and a bring-up after
  `ReconnectCheck::Lost`. The sink's `TMDS_Config` written again with the output running
  and no wait, the source's scrambler set again to the link's, and bring-up's scrambling
  check on a scrambled link. It reads `tmds_config_write`, `on_tmds_config_failure` and the
  check's timing from `BringUpConfig`. `Restored` carries the write's time, the check and a
  failed write it carried on past (`write_failed`); `RestoreError` is `Scdc` (a
  non-transient error, or any under `Stop`) or `Phy` (the scrambler call), with no
  tear-down. The cancel flag is read by the check alone, so a cancel leaves the link
  running with the check `Cancelled`. Its events (`QuickRestoreStarted`,
  `QuickRestoreEnded`) and `MAX_QUICK_RESTORE_EVENTS`.
- **`BringUpConfig::validate`**: reports, in a `ConfigError`, a scrambling deadline not
  longer than the ratio wait (every bring-up would miss it), a check interval not shorter
  than the check window (the check reads once) and a check interval of 0 (back-to-back
  reads). The procedures still run with any values.

### Changed

- **The README, crate docs and roadmap describe the procedures**: the README shows a
  bring-up and tear-down over the four parts (compiled as a doctest), lists the `culvert`
  feature, and no longer says the crate has no dependencies without `display-types`; the
  roadmap keeps what is still to come.
- **`doc/architecture.md` describes the procedures**: each step with its settings, defaults
  and sources, cancellation, the settings and result types, `TmdsScdc` and the culvert
  adapter, the events and their bounds, and what the procedures use from hdmi-hal and how a
  caller drives them. The link choice is brought up to date (the source's scrambler and
  maximum rate, the `TMDS_Config` gate, hdmi-hal's clock ratio, the dependencies), with
  each reason a user needs next to the rule it explains.
- **The README shows the "trusted publishing only" badge**: crates.io refuses token
  uploads for this crate, and "Verifying releases" says so.

### Internal

- CI and the publish workflow's verify job also run the `culvert` adapter's tests and build
  the docs with every feature, as docs.rs does; the MSRV check and the coverage measurement
  include the `culvert` feature.
- CI fails on `main`, and the publish workflow before anything is built, when `Cargo.toml`
  has a `[patch]` section: `cargo package` drops it, so a release built with one would not
  build from crates.io.
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
