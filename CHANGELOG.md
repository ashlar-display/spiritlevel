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
  and hashes the sink facts, source facts and policy it was chosen from.
  `NONE == new(false, false, false)` holds as in 0.1.

### Added

- **The procedures**, written once as an I/O-free core (`procedure::TmdsIo` and the `async fn`s
  over it) and driven synchronously by `TmdsLink` over a `TmdsScdc`, a `TmdsPhy`, a `Clock`
  and a `Delay`, with no runtime. Each makes one attempt per call; retries are the caller's.
  - **Bring-up** (`bring_up`): on a sink with SCDC, the FRL clear and `Source_Version` first,
    each a warning when it fails, whatever the error's kind; then the output stopped, the
    sink's `TMDS_Config` written (read-modify-write or whole byte; skipped where no write is
    due, with `TmdsConfigSkip`), the PHY configured with its scrambler, the ratio wait spent
    from the write, and the output started; a video whose TMDS character rate is not the
    link's is refused first, before any I/O (`BringUpError::VideoMismatch`). With
    `OutputDuringSinkWrite::Running`, unscrambled output runs through the write and the
    scrambler comes on after the wait. A
    failed `TMDS_Config` write carries on as a warning under `CarryOn`, with the register
    read back on a plain link; a missed deadline is a warning. A PHY failure, a
    non-transient error of the `TMDS_Config` write or a failed write under `Stop` ends it
    with `BringUpError`, after a tear-down unless `tear_down_on_error` is off (with the sink
    taken as gone after `Detached`).
  - **The scrambling check**, on a scrambled link: `Scrambling_Status` read every
    `check_interval` until the sink reports scrambling or `check_window` ends, from the
    output's start (from the scrambler with `Running`), through failed reads, stopping early
    only on `Detached` or `Unsupported`, and never starting a read after the window. It ends
    `Set`, `NotSet` or `Unreadable` in `LinkUp::check`; the last two are also a
    `ScramblingNotConfirmed` warning. A read count of `ceil(window / interval) + 1` ends it
    on a clock that does not advance (`read_cap_reached`).
  - **Tear-down** (`tear_down`): the output stopped; with `clear_sink`, the sink's
    `TMDS_Config` written to 0 and the scrambler turned off first, in the order
    `SinkClearTiming` and `ScramblerOff` give, writing nothing to a plain link, a sink without
    SCDC or a sink the caller says is gone (`SinkPresence`). Every step is attempted: a failed
    sink write is a warning, a failed PHY call still lets the output stop.
  - **The reconnect check** (`check_reconnect`): after a hotplug while the link is up, one
    read of the sink's `TMDS_Config`, compared with what the link needs on both bits, a plain
    link included. `ReconnectCheck` is `Matches`, `Lost` (with the reading), `Unreadable`,
    `NotApplicable` (no write due) or `SinkChanged`: the link no longer fits the caller's
    current sink facts, and nothing is read. It writes nothing.
  - **The quick restore** (`quick_restore`): the option to a tear-down and a bring-up after
    `Lost`: the sink's `TMDS_Config` written again with the output running (after `Lost` on a
    1/40 link the sink then switches its clock ratio with TMDS running, as Xilinx and Amlogic
    do), the source's scrambler set again to the link's, and the scrambling check on a
    scrambled link. `Restored` and `RestoreError` (`Scdc` or `Phy`, with no tear-down).
- **Cancellation** through hdmi-hal's `Cancel`, re-exported, implemented for `AtomicBool`
  and any reference to a flag, with `NoCancel` the default (`TmdsLink::with_cancel`). Bring-up reads the flag at entry, before each SCDC step,
  before each slice of the ratio wait (slices of at most `check_interval`) and before each
  read of the check, and ends `BringUpOutcome::Cancelled` with where it was seen
  (`CancelPoint`): at entry before any I/O, otherwise after a tear-down. Tear-down and the
  reconnect check are not cancelled, the quick restore only in its check, and the flag is
  never cleared.
- **The results**: `BringUpOutcome` (`Up` with a `LinkUp`, or `Cancelled`), `BringUpError`
  (`Scdc` or `Phy`, with the step, the output's state, the `Cleanup` and the warnings
  gathered), the seven `BringUpWarning` kinds in fixed `Warnings` storage (`iter_warnings`),
  `ScramblingCheck` with `CheckResult` and `FailedRead`, `OutputState`, `TornDown`,
  `TearDownError` (the first failure of each step) with `SinkClear`, `ReconnectCheck`,
  `Restored` and `RestoreError`. A failed read-and-write is an `UpdateError` in every value,
  and an error or event names its half by its `TmdsStep` (`ReadTmdsConfig` or
  `WriteTmdsConfig`). `BringUpError`, `TearDownError` and `RestoreError` implement
  `Display` and `core::error::Error`, with the SCDC error as the `source`.
- **Diagnostics**: each procedure's `_with_events` form calls a closure with every step as a
  `TmdsEvent` (a `Timestamp` from the procedure's clock and a `TmdsEventKind`), `Copy` and
  without type parameters: a failed step is recorded by its `TmdsStep` and, for SCDC, its
  `DdcErrorKind`, never by the error value; a skipped step by its rule (`FrlClearSkip`,
  `SourceVersionSkip`, `TmdsConfigSkip`, `SinkClearSkip`). `MAX_BRING_UP_EVENTS`,
  `MAX_TEAR_DOWN_EVENTS`, `MAX_RECONNECT_CHECK_EVENTS` and `MAX_QUICK_RESTORE_EVENTS` bound
  how many each records.
- **The settings**: `BringUpConfig` (the ratio wait, the scrambling deadline, the check's
  window and interval, `ClearFrl`, `SourceVersionWrite`, `OutputDuringSinkWrite`,
  `TmdsConfigWrite`, `OnTmdsConfigFailure`, `tear_down_on_error` and the `TearDownConfig`
  used after an error or a cancel) and `TearDownConfig` (`clear_sink`, `SinkClearTiming`,
  `ScramblerOff`), each with a `DEFAULT` and `const fn` setters.
  `BringUpConfig::validate` reports, in a `ConfigError`, a deadline not longer than the wait,
  a check interval not shorter than the window, and an interval of 0; the procedures run
  with any values.
- **`TmdsScdc`**, the sink-side SCDC operations the procedures make: `TMDS_Config` written
  whole or read-modify-written, read back (as a `TmdsConfigReading`, with the byte read),
  `Scrambling_Status` (`ScramblingStatus`, with the byte read), `Sink_Version`,
  `Source_Version`, and the FRL clear (`clear_frl_rate`, `clear_flt_update`). Its error is a
  `DdcError`, so a procedure can tell a failure that will pass from one that will not;
  `UpdateError` says which half of a read-and-write failed, and is a `DdcError` too.
  Implemented for `&mut T`, so a caller can lend its client and keep it.
- **The `culvert` feature: `TmdsScdc` for culvert's client** (`culvert::Scdc<T>`, over any
  `ScdcTransport`), with culvert's errors wrapped in `CulvertError`, whose kind is the
  transport's own, or `Unsupported` for a request culvert refuses; it is `Copy` and
  `PartialEq` as culvert's error is, so the outcomes stay so, and prints and chains as it. The reserved bits survive
  culvert's own read-modify-write, and the bytes read reach `TmdsConfigReading::raw` and
  `ScramblingStatus::raw`.
- **`TmdsLink`, the sync driver**: the four procedures with their `_with_events` forms,
  `into_parts`, and `scdc`, `scdc_mut`, `phy`, `phy_mut` and `clock` to reach the parts
  between procedures.
- **Choosing the link, extended**:
  - `SourceTmdsFacts::scrambling` and `max_character_rate_khz`, filled by
    `From<&TmdsPhyCapabilities>`. A source without a scrambler is treated as a sink that
    cannot take scrambling (refused above 340 000 kHz with
    `ScramblingUnsupported::source_cannot_scramble`, the plain link at exactly 340 000 kHz
    under `AtOrAbove340`, `ScramblingOff::source_cannot`); a rate above the source's maximum
    is refused first, with `ParamsError::AboveSourceMaximum`. `new` takes the source to have
    a scrambler and no maximum, so 0.1's results do not change.
  - `SinkTmdsFacts::frl_capable`, from a non-zero `Max_FRL_Rate`, with `with_frl_capable`;
    `new` sets it for a sink with SCDC, so the FRL clear is not skipped by a caller that never
    learns of it.
  - `TmdsPolicy::tmds_config_gate`: which sinks get the `TMDS_Config` write on a plain link,
    `TmdsConfigGate::AnyScdcSink` (the default, 0.1's rule) or `ScramblingSink`;
    `sink_config()` is `None` exactly when no write is due.
  - `TmdsLinkParams::sink()`, the sink facts a link was chosen from, and `fits`, whether
    choosing again for other sink facts gives the same ratio, scrambling and `TMDS_Config`.
- **Re-exports of the types the API uses**, so `spiritlevel` is a caller's or an
  implementation's only dependency and names the same versions: from hdmi-hal `TmdsPhy`,
  `TmdsPhyCapabilities`, `TmdsPhyConfig`, `TmdsVideo`, `ColorBitDepth`, `ColorFormat`,
  `Clock`, `Delay`, `Timestamp`, `DdcError`, `DdcErrorKind`, `NoAcknowledgeSource`,
  `Cancel` and `NoCancel`; with `display-types`, `HdmiForumSinkCap`; with `culvert`,
  culvert's `Scdc`, `ScdcError`, `ProtocolError` and `ScdcTransport`.

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
