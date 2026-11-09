//! Bring-up: start the TMDS link [`TmdsLinkParams`] describes, in stages: the work before
//! both paths (the FRL clear and `Source_Version`), then the main path (output, the sink's
//! `TMDS_Config`, the PHY, the wait, output on).

use core::time::Duration;

use hdmi_hal::ddc::{DdcError, DdcErrorKind};
use hdmi_hal::phy::{TmdsPhyConfig, TmdsVideo};
use hdmi_hal::time::Timestamp;

use crate::config::{
    BringUpConfig, ClearFrl, OnTmdsConfigFailure, OutputDuringSinkWrite, SinkPresence,
    SourceVersionWrite, TmdsConfigWrite,
};
use crate::error::UpdateError;
use crate::event::{
    CancelPoint, FrlClearSkip, SourceVersionSkip, TmdsConfigSkip, TmdsEvent, TmdsEventKind,
    TmdsStep,
};
use crate::outcome::{
    BringUpError, BringUpOutcome, BringUpWarning, CheckResult, Cleanup, LinkUp, OutputState,
    ScramblingCheck, Warnings,
};
use crate::params::TmdsLinkParams;

use super::check::scrambling_check;
use super::{TmdsIo, tear_down};

/// A step that stopped the bring-up, and its error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stop<S> {
    pub(crate) step: TmdsStep,
    pub(crate) error: S,
}

/// Whether an error may pass: everything but a bus that is gone (`Detached`) or a request
/// that cannot be done (`Unsupported`). A kind added later counts as transient.
pub(crate) fn transient(kind: DdcErrorKind) -> bool {
    !matches!(kind, DdcErrorKind::Detached | DdcErrorKind::Unsupported)
}

/// A failed SCDC step before both paths: a warning, whatever the error's kind.
fn failed<Io: TmdsIo, E: DdcError>(
    io: &Io,
    emit: &mut impl FnMut(&Io, TmdsEventKind),
    step: TmdsStep,
    error: E,
    warning: impl FnOnce(E) -> BringUpWarning<Io::ScdcError>,
    warnings: &mut Warnings<Io::ScdcError>,
) {
    let kind = error.kind();
    emit(
        io,
        TmdsEventKind::ScdcStepFailed {
            step,
            kind,
            ends: false,
        },
    );
    warnings.push(warning(error));
}

/// Why the work before both paths or the sink's write ended early: a step's stop, or the
/// cancel flag.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Halt<S> {
    Stop(Stop<S>),
    Cancelled(CancelPoint),
}

impl<S> From<Stop<S>> for Halt<S> {
    fn from(stop: Stop<S>) -> Self {
        Self::Stop(stop)
    }
}

impl<S> From<CancelPoint> for Halt<S> {
    fn from(at: CancelPoint) -> Self {
        Self::Cancelled(at)
    }
}

impl<S> Halt<S> {
    /// The failure, with the output's state at a stop.
    fn into_failure<P>(self, output: OutputState) -> Failure<S, P> {
        match self {
            Self::Stop(stop) => Failure::Scdc(stop, output),
            Self::Cancelled(at) => Failure::Cancelled(at),
        }
    }
}

/// `Err(at)` when the caller's cancel flag is set.
fn checkpoint<Io: TmdsIo>(io: &Io, at: CancelPoint) -> Result<(), CancelPoint> {
    if io.cancelled() { Err(at) } else { Ok(()) }
}

/// The work before both paths, in order: the FRL clear (`Config_1`, then `FLT_update`) when
/// the settings give it, then `Source_Version`. Nothing on a sink without SCDC. A failed
/// step is a warning whatever the error's kind, as no source treats one as fatal: a bus
/// that is gone (`Detached`) fails the `TMDS_Config` write that follows, which stops the
/// bring-up, and a request the implementation cannot do (`Unsupported`) does not block the
/// link. The cancel flag is read before each step.
pub(crate) async fn before_both_paths<Io: TmdsIo>(
    io: &mut Io,
    link: &TmdsLinkParams,
    config: &BringUpConfig,
    warnings: &mut Warnings<Io::ScdcError>,
    emit: &mut impl FnMut(&Io, TmdsEventKind),
) -> Result<(), Halt<Io::ScdcError>> {
    let sink = link.sink();

    let clear = match config.clear_frl {
        _ if !sink.scdc_present => Err(FrlClearSkip::NoScdc),
        ClearFrl::EveryScdcSink => Ok(()),
        ClearFrl::IfSinkDeclaresFrl if sink.frl_capable => Ok(()),
        _ => Err(FrlClearSkip::SinkDeclaresNoFrl),
    };
    match clear {
        Err(reason) => emit(io, TmdsEventKind::FrlClearSkipped { reason }),
        Ok(()) => {
            checkpoint(io, CancelPoint::Step(TmdsStep::ClearFrlRate))?;
            match io.clear_frl_rate().await {
                Ok(()) => emit(io, TmdsEventKind::FrlRateCleared),
                Err(e) => failed(
                    io,
                    emit,
                    TmdsStep::ClearFrlRate,
                    e,
                    BringUpWarning::FrlRateClearFailed,
                    warnings,
                ),
            }
            checkpoint(io, CancelPoint::Step(TmdsStep::ClearFltUpdate))?;
            match io.clear_flt_update().await {
                Ok(was_set) => emit(io, TmdsEventKind::FltUpdateCleared { was_set }),
                Err(e) => failed(
                    io,
                    emit,
                    TmdsStep::ClearFltUpdate,
                    e,
                    BringUpWarning::FltUpdateClearFailed,
                    warnings,
                ),
            }
        }
    }

    let skip = |reason| TmdsEventKind::SourceVersionSkipped { reason };
    if !sink.scdc_present {
        emit(io, skip(SourceVersionSkip::NoScdc));
        return Ok(());
    }
    let reads_version = match config.source_version {
        SourceVersionWrite::EveryBringUp => false,
        SourceVersionWrite::IfSinkVersion1 => true,
        SourceVersionWrite::WhenScrambling if link.scrambling() => true,
        SourceVersionWrite::WhenScrambling => {
            emit(io, skip(SourceVersionSkip::NotScrambled));
            return Ok(());
        }
        _ => {
            emit(io, skip(SourceVersionSkip::Never));
            return Ok(());
        }
    };
    if reads_version {
        checkpoint(io, CancelPoint::Step(TmdsStep::SinkVersion))?;
        let version = match io.read_sink_version().await {
            Ok(version) => version,
            Err(e) => {
                failed(
                    io,
                    emit,
                    TmdsStep::SinkVersion,
                    e,
                    BringUpWarning::SinkVersionReadFailed,
                    warnings,
                );
                emit(io, skip(SourceVersionSkip::SinkVersionUnread));
                return Ok(());
            }
        };
        emit(io, TmdsEventKind::SinkVersionRead { version });
        let due = match config.source_version {
            SourceVersionWrite::IfSinkVersion1 => version == 1,
            _ => version != 0,
        };
        if !due {
            emit(io, skip(SourceVersionSkip::SinkVersion { version }));
            return Ok(());
        }
    }
    checkpoint(io, CancelPoint::Step(TmdsStep::SourceVersion))?;
    match io.write_source_version(1).await {
        Ok(()) => emit(io, TmdsEventKind::SourceVersionWritten { version: 1 }),
        Err(e) => failed(
            io,
            emit,
            TmdsStep::SourceVersion,
            e,
            BringUpWarning::SourceVersionWriteFailed,
            warnings,
        ),
    }
    Ok(())
}

/// Why a bring-up stopped, before the cleanup: a step, its error, and the output's state;
/// or the cancel flag, and where it was seen.
enum Failure<S, P> {
    Scdc(Stop<S>, OutputState),
    Phy(TmdsStep, P, OutputState),
    Cancelled(CancelPoint),
}

impl<S, P> From<CancelPoint> for Failure<S, P> {
    fn from(at: CancelPoint) -> Self {
        Self::Cancelled(at)
    }
}

/// What the main path produced, for `LinkUp`.
struct Up<S> {
    sink_written_at: Option<Timestamp>,
    output_started_at: Timestamp,
    check: ScramblingCheck<S>,
}

/// Records `kind` at the I/O's time.
fn emit<Io: TmdsIo, F: FnMut(TmdsEvent)>(io: &Io, record: &mut F, kind: TmdsEventKind) {
    record(TmdsEvent::new(io.now(), kind));
}

/// A PHY call's error as a failure, recorded.
fn phy_failed<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &Io,
    record: &mut F,
    step: TmdsStep,
    error: Io::PhyError,
    output: OutputState,
) -> Failure<Io::ScdcError, Io::PhyError> {
    emit(
        io,
        record,
        TmdsEventKind::PhyStepFailed { step, ends: true },
    );
    Failure::Phy(step, error, output)
}

/// Writes the sink's `TMDS_Config` when a write is due: the time of the write (`None` when
/// none was due, or when the read of a read-modify-write failed, so nothing was written),
/// or the stop. A transient failure under `CarryOn` is a warning; on a plain
/// link it reads the register back. The cancel flag is read before the write and the read.
async fn write_sink<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    link: &TmdsLinkParams,
    config: &BringUpConfig,
    warnings: &mut Warnings<Io::ScdcError>,
    record: &mut F,
) -> Result<Option<Timestamp>, Halt<Io::ScdcError>> {
    let Some(value) = link.sink_config() else {
        let reason = if link.sink().scdc_present {
            TmdsConfigSkip::Gate
        } else {
            TmdsConfigSkip::NoScdc
        };
        emit(io, record, TmdsEventKind::TmdsConfigSkipped { reason });
        return Ok(None);
    };
    checkpoint(io, CancelPoint::Step(TmdsStep::WriteTmdsConfig))?;
    let result = match config.tmds_config_write {
        TmdsConfigWrite::WholeByte => io
            .write_tmds_config(value)
            .await
            .map_err(UpdateError::Write),
        _ => io.update_tmds_config(value).await,
    };
    let written_at = io.now();
    let error = match result {
        Ok(()) => {
            emit(
                io,
                record,
                TmdsEventKind::TmdsConfigWritten { config: value },
            );
            return Ok(Some(written_at));
        }
        Err(failure) => failure,
    };
    // The step names the half: a read-modify-write whose read failed wrote nothing.
    let step = match error {
        UpdateError::Read(_) => TmdsStep::ReadTmdsConfig,
        UpdateError::Write(_) => TmdsStep::WriteTmdsConfig,
    };
    let kind = error.kind();
    let ends = !transient(kind) || config.on_tmds_config_failure == OnTmdsConfigFailure::Stop;
    let wrote = matches!(error, UpdateError::Write(_));
    emit(
        io,
        record,
        TmdsEventKind::ScdcStepFailed { step, kind, ends },
    );
    if ends {
        return Err(Stop {
            step,
            error: error.into_inner(),
        }
        .into());
    }
    // A plain link has no scrambling check: read the register back so a sink that kept
    // 1/40 after a failed clear does not go unnoticed.
    let read_back = if link.scrambling() {
        None
    } else {
        if io.cancelled() {
            warnings.push(BringUpWarning::TmdsConfigWriteFailed {
                error,
                read_back: None,
            });
            return Err(CancelPoint::Step(TmdsStep::ReadTmdsConfig).into());
        }
        let read = io.read_tmds_config().await;
        match &read {
            Ok(reading) => emit(
                io,
                record,
                TmdsEventKind::TmdsConfigRead { reading: *reading },
            ),
            Err(e) => emit(
                io,
                record,
                TmdsEventKind::ScdcStepFailed {
                    step: TmdsStep::ReadTmdsConfig,
                    kind: e.kind(),
                    ends: false,
                },
            ),
        }
        Some(read)
    };
    warnings.push(BringUpWarning::TmdsConfigWriteFailed { error, read_back });
    // A failed write may have reached the sink, so the wait and the deadline run from it;
    // a failed read wrote nothing.
    Ok(wrote.then_some(written_at))
}

/// Spends what is left of the ratio wait after the write at `written_at`; nothing when no
/// write was made. The wait is spent in slices of at most `slice` (the check interval;
/// one slice when it is 0), with the cancel flag read before each, so a cancel is seen
/// within one interval whatever the wait; with an interval of 0, only before the wait.
async fn wait_after<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    written_at: Option<Timestamp>,
    wait: Duration,
    slice: Duration,
    record: &mut F,
) -> Result<(), CancelPoint> {
    let Some(written_at) = written_at else {
        return Ok(());
    };
    let left = wait.saturating_sub(io.now().saturating_duration_since(written_at));
    let slice = if slice.is_zero() { left } else { slice };
    let mut remaining = left;
    loop {
        checkpoint(io, CancelPoint::RatioWait)?;
        if remaining.is_zero() {
            break;
        }
        let next = remaining.min(slice);
        io.delay(next).await;
        remaining -= next;
    }
    emit(io, record, TmdsEventKind::WaitSpent { duration: left });
    Ok(())
}

/// The main path in the default order: output off, the sink's write, the PHY with its
/// scrambler, the wait, output on.
async fn main_path<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    link: &TmdsLinkParams,
    phy: TmdsPhyConfig,
    config: &BringUpConfig,
    warnings: &mut Warnings<Io::ScdcError>,
    record: &mut F,
) -> Result<Up<Io::ScdcError>, Failure<Io::ScdcError, Io::PhyError>> {
    let scrambled = link.scrambling();
    let running = config.output_during_sink_write == OutputDuringSinkWrite::Running;

    if let Err(e) = io.stop_output().await {
        return Err(phy_failed(
            io,
            record,
            TmdsStep::StopOutput,
            e,
            OutputState::MayBeRunning,
        ));
    }
    emit(io, record, TmdsEventKind::OutputStopped);

    if !running {
        let written_at = write_sink(io, link, config, warnings, record)
            .await
            .map_err(|halt| halt.into_failure(OutputState::Stopped))?;
        configure(io, phy, record).await?;
        set_scrambling(io, scrambled, OutputState::Stopped, record).await?;
        wait_after(
            io,
            written_at,
            config.ratio_wait,
            config.check_interval,
            record,
        )
        .await?;
        let started = start(io, written_at, record).await?;
        deadline(io, written_at, started, scrambled, config, warnings, record);
        let check = check_from(io, started, scrambled, config, warnings, record).await?;
        return Ok(Up {
            sink_written_at: written_at,
            output_started_at: started,
            check,
        });
    }

    // `Running`: unscrambled output first, then the sink's write, the wait, and only then
    // the source's scrambler, so no scrambled TMDS reaches the sink before it is told.
    configure(io, phy, record).await?;
    set_scrambling(io, false, OutputState::Stopped, record).await?;
    let started = start(io, None, record).await?;
    let written_at = write_sink(io, link, config, warnings, record)
        .await
        .map_err(|halt| halt.into_failure(OutputState::MayBeRunning))?;
    wait_after(
        io,
        written_at,
        config.ratio_wait,
        config.check_interval,
        record,
    )
    .await?;
    let mut check = ScramblingCheck::NOT_APPLICABLE;
    if scrambled {
        set_scrambling(io, true, OutputState::MayBeRunning, record).await?;
        let scrambling_at = io.now();
        deadline(
            io,
            written_at,
            scrambling_at,
            scrambled,
            config,
            warnings,
            record,
        );
        check = check_from(io, scrambling_at, scrambled, config, warnings, record).await?;
    }
    Ok(Up {
        sink_written_at: written_at,
        output_started_at: started,
        check,
    })
}

async fn configure<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    phy: TmdsPhyConfig,
    record: &mut F,
) -> Result<(), Failure<Io::ScdcError, Io::PhyError>> {
    match io.configure(phy).await {
        Ok(()) => {
            emit(
                io,
                record,
                TmdsEventKind::PhyConfigured {
                    character_rate_khz: phy.character_rate_khz,
                    clock_ratio: phy.clock_ratio,
                },
            );
            Ok(())
        }
        Err(e) => Err(phy_failed(
            io,
            record,
            TmdsStep::Configure,
            e,
            OutputState::Stopped,
        )),
    }
}

async fn set_scrambling<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    enabled: bool,
    output: OutputState,
    record: &mut F,
) -> Result<(), Failure<Io::ScdcError, Io::PhyError>> {
    match io.set_scrambling(enabled).await {
        Ok(()) => {
            emit(io, record, TmdsEventKind::ScramblerSet { enabled });
            Ok(())
        }
        Err(e) => Err(phy_failed(io, record, TmdsStep::SetScrambling, e, output)),
    }
}

async fn start<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    written_at: Option<Timestamp>,
    record: &mut F,
) -> Result<Timestamp, Failure<Io::ScdcError, Io::PhyError>> {
    if let Err(e) = io.start_output().await {
        return Err(phy_failed(
            io,
            record,
            TmdsStep::StartOutput,
            e,
            OutputState::MayBeRunning,
        ));
    }
    let started = io.now();
    let after_write = written_at.map(|w| started.saturating_duration_since(w));
    emit(io, record, TmdsEventKind::OutputStarted { after_write });
    Ok(started)
}

/// The scrambling check from `from`, on a scrambled link; a plain link's check does not
/// apply. A check that did not see scrambling is also a warning; a cancelled check is the
/// bring-up's cancellation.
async fn check_from<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    from: Timestamp,
    scrambled: bool,
    config: &BringUpConfig,
    warnings: &mut Warnings<Io::ScdcError>,
    record: &mut F,
) -> Result<ScramblingCheck<Io::ScdcError>, CancelPoint> {
    if !scrambled {
        return Ok(ScramblingCheck::NOT_APPLICABLE);
    }
    let check =
        scrambling_check(io, from, config.check_window, config.check_interval, record).await;
    match check.result {
        CheckResult::Cancelled => return Err(CancelPoint::Check),
        result @ (CheckResult::NotSet | CheckResult::Unreadable) => {
            warnings.push(BringUpWarning::ScramblingNotConfirmed { result });
        }
        _ => {}
    }
    Ok(check)
}

/// A warning when scrambled output reached the sink later after its write than the
/// deadline allows. A plain link has no deadline.
fn deadline<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &Io,
    written_at: Option<Timestamp>,
    scrambled_from: Timestamp,
    scrambled: bool,
    config: &BringUpConfig,
    warnings: &mut Warnings<Io::ScdcError>,
    record: &mut F,
) {
    let Some(written_at) = written_at.filter(|_| scrambled) else {
        return;
    };
    let took = scrambled_from.saturating_duration_since(written_at);
    if took > config.scrambling_deadline {
        let by = took - config.scrambling_deadline;
        emit(io, record, TmdsEventKind::DeadlineMissed { by });
        warnings.push(BringUpWarning::DeadlineMissed { by });
    }
}

/// Brings up the TMDS link `link` describes, carrying `video`.
///
/// One attempt: the work before both paths (the FRL clear and `Source_Version`, on a sink
/// with SCDC), then the main path. By default (`OutputDuringSinkWrite::Stopped`) the output
/// is stopped, the sink's `TMDS_Config` written (when a write is due), the PHY configured with
/// its scrambler, the ratio wait spent from the write, and the output started. With `Running`,
/// unscrambled output runs through the write and the source's scrambler comes on after the
/// wait. No scrambled TMDS reaches the sink before its write in either order.
///
/// What carries on is a warning in the outcome: a failed FRL clear or `Source_Version`, a
/// failed `TMDS_Config` write under `OnTmdsConfigFailure::CarryOn` (on a plain link with the
/// register read back), a missed deadline. Whether the sink reports scrambling is the
/// check's, part of the outcome rather than a condition of it.
///
/// # Cancellation
///
/// The caller's cancel flag is read at entry, before each SCDC step, before each slice of
/// the ratio wait (slices of at most `check_interval`) and before each read of the check;
/// never mid-call. Once set, the bring-up ends [`BringUpOutcome::Cancelled`]: at entry
/// before any I/O, otherwise after a tear-down with the caller's `tear_down` settings
/// (`tear_down_on_error` governs errors only). The tear-down is not cancelled, and the flag
/// is never cleared: the caller resets it before its next call.
///
/// Records at most [`MAX_BRING_UP_EVENTS`](crate::MAX_BRING_UP_EVENTS) events.
///
/// # Errors
///
/// [`BringUpError`] when a PHY call failed, or the sink's `TMDS_Config` write failed with a
/// non-transient error (`Detached`, `Unsupported`) or under `OnTmdsConfigFailure::Stop`. A
/// failure before both paths, or of a plain link's read-back, is a warning.
/// With `tear_down_on_error` (the default) a tear-down follows, with the sink taken to be
/// gone when the error was `Detached`; its result is the error's [`Cleanup`].
pub async fn bring_up<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    link: &TmdsLinkParams,
    video: TmdsVideo,
    config: &BringUpConfig,
    record: &mut F,
) -> Result<BringUpOutcome<Io::ScdcError, Io::PhyError>, BringUpError<Io::ScdcError, Io::PhyError>>
{
    emit(
        io,
        record,
        TmdsEventKind::BringUpStarted {
            link: *link,
            video,
            config: *config,
        },
    );
    let mut warnings = Warnings::new();
    if io.cancelled() {
        let at = CancelPoint::Entry;
        emit(io, record, TmdsEventKind::BringUpCancelled { at });
        emit(
            io,
            record,
            TmdsEventKind::BringUpEnded {
                failed_at: None,
                cancelled: true,
            },
        );
        return Ok(BringUpOutcome::Cancelled {
            at,
            cleanup: Cleanup::Skipped,
            warnings,
        });
    }
    let phy = TmdsPhyConfig::new(link.rate_khz(), link.clock_ratio(), video);

    let result = match before_both_paths(io, link, config, &mut warnings, &mut |io, kind| {
        emit(io, record, kind);
    })
    .await
    {
        Ok(()) => main_path(io, link, phy, config, &mut warnings, record).await,
        Err(halt) => Err(halt.into_failure(OutputState::MayBeRunning)),
    };

    let failure = match result {
        Ok(up) => {
            emit(
                io,
                record,
                TmdsEventKind::BringUpEnded {
                    failed_at: None,
                    cancelled: false,
                },
            );
            return Ok(BringUpOutcome::Up(LinkUp {
                link: *link,
                sink_written_at: up.sink_written_at,
                output_started_at: up.output_started_at,
                check: up.check,
                warnings,
            }));
        }
        Err(Failure::Cancelled(at)) => {
            emit(io, record, TmdsEventKind::BringUpCancelled { at });
            let torn = tear_down(io, link, &config.tear_down, SinkPresence::Present, record).await;
            emit(
                io,
                record,
                TmdsEventKind::BringUpEnded {
                    failed_at: None,
                    cancelled: true,
                },
            );
            return Ok(BringUpOutcome::Cancelled {
                at,
                cleanup: Cleanup::TornDown(torn),
                warnings,
            });
        }
        Err(Failure::Scdc(Stop { step, error }, output)) => {
            let presence = if error.kind() == DdcErrorKind::Detached {
                SinkPresence::Gone
            } else {
                SinkPresence::Present
            };
            let error = BringUpError::Scdc {
                step,
                error,
                output,
                cleanup: Cleanup::Skipped,
                warnings,
            };
            (step, presence, error)
        }
        Err(Failure::Phy(step, error, output)) => {
            let error = BringUpError::Phy {
                step,
                error,
                output,
                cleanup: Cleanup::Skipped,
                warnings,
            };
            (step, SinkPresence::Present, error)
        }
    };
    let (step, presence, mut error) = failure;

    if config.tear_down_on_error {
        let torn = tear_down(io, link, &config.tear_down, presence, record).await;
        match &mut error {
            BringUpError::Scdc { cleanup, .. } | BringUpError::Phy { cleanup, .. } => {
                *cleanup = Cleanup::TornDown(torn);
            }
        }
    }
    emit(
        io,
        record,
        TmdsEventKind::BringUpEnded {
            failed_at: Some(step),
            cancelled: false,
        },
    );
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedure::test_io::{Op, TestIo, block_on};
    use crate::types::{SinkTmdsFacts, SourceTmdsFacts, TmdsPolicy};

    /// An HDMI 2.1 sink with SCDC that declares FRL.
    const FRL_SINK: SinkTmdsFacts = SinkTmdsFacts::new(true, true, false);
    const NO_FRL: SinkTmdsFacts = FRL_SINK.with_frl_capable(false);

    fn link(rate_khz: u32, sink: SinkTmdsFacts) -> TmdsLinkParams {
        TmdsLinkParams::choose(
            rate_khz,
            sink,
            SourceTmdsFacts::new(false),
            TmdsPolicy::DEFAULT,
        )
        .unwrap()
    }

    struct Run {
        result: Result<(), Halt<DdcErrorKind>>,
        warnings: Warnings<DdcErrorKind>,
        events: [Option<TmdsEventKind>; 8],
        count: usize,
    }

    impl Run {
        fn has(&self, kind: TmdsEventKind) -> bool {
            self.events[..self.count].contains(&Some(kind))
        }
    }

    fn run(io: &mut TestIo, link: &TmdsLinkParams, config: &BringUpConfig) -> Run {
        let mut warnings = Warnings::new();
        let mut events = [None; 8];
        let mut count = 0;
        let result = block_on(before_both_paths(
            io,
            link,
            config,
            &mut warnings,
            &mut |_, kind| {
                events[count] = Some(kind);
                count += 1;
            },
        ));
        Run {
            result,
            warnings,
            events,
            count,
        }
    }

    #[test]
    fn an_frl_sink_is_cleared_and_gets_source_version_by_default() {
        let mut io = TestIo::new();
        io.flt_update = true;
        let r = run(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
        assert_eq!(r.result, Ok(()));
        assert_eq!(
            io.ops(),
            [
                Op::ClearFrlRate,
                Op::ClearFltUpdate,
                Op::WriteSourceVersion(1)
            ]
        );
        assert!(r.has(TmdsEventKind::FrlRateCleared));
        assert!(r.has(TmdsEventKind::FltUpdateCleared { was_set: true }));
        assert!(r.has(TmdsEventKind::SourceVersionWritten { version: 1 }));
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn the_frl_clear_follows_the_setting() {
        let mut io = TestIo::new();
        let r = run(&mut io, &link(594_000, NO_FRL), &BringUpConfig::DEFAULT);
        assert!(r.has(TmdsEventKind::FrlClearSkipped {
            reason: FrlClearSkip::SinkDeclaresNoFrl
        }));
        assert_eq!(io.ops(), [Op::WriteSourceVersion(1)]);

        let every = BringUpConfig::DEFAULT.with_clear_frl(ClearFrl::EveryScdcSink);
        let mut io = TestIo::new();
        run(&mut io, &link(594_000, NO_FRL), &every);
        assert_eq!(
            io.ops(),
            [
                Op::ClearFrlRate,
                Op::ClearFltUpdate,
                Op::WriteSourceVersion(1)
            ]
        );
    }

    #[test]
    fn a_sink_without_scdc_gets_nothing() {
        let mut io = TestIo::new();
        let every = BringUpConfig::DEFAULT.with_clear_frl(ClearFrl::EveryScdcSink);
        let r = run(&mut io, &link(148_500, SinkTmdsFacts::NONE), &every);
        assert_eq!(r.result, Ok(()));
        assert_eq!(io.ops().len(), 0);
        assert!(r.has(TmdsEventKind::FrlClearSkipped {
            reason: FrlClearSkip::NoScdc
        }));
        assert!(r.has(TmdsEventKind::SourceVersionSkipped {
            reason: SourceVersionSkip::NoScdc
        }));
    }

    #[test]
    fn source_version_follows_the_setting() {
        let only = |w| {
            BringUpConfig::DEFAULT
                .with_clear_frl(ClearFrl::IfSinkDeclaresFrl)
                .with_source_version(w)
        };
        let scrambled = link(594_000, NO_FRL);
        let plain = link(148_500, NO_FRL);

        // Only when Sink_Version is 1.
        let mut io = TestIo::new();
        io.sink_version = 1;
        run(
            &mut io,
            &scrambled,
            &only(SourceVersionWrite::IfSinkVersion1),
        );
        assert_eq!(io.ops(), [Op::ReadSinkVersion, Op::WriteSourceVersion(1)]);
        let mut io = TestIo::new();
        io.sink_version = 2;
        let r = run(
            &mut io,
            &scrambled,
            &only(SourceVersionWrite::IfSinkVersion1),
        );
        assert_eq!(io.ops(), [Op::ReadSinkVersion]);
        assert!(r.has(TmdsEventKind::SourceVersionSkipped {
            reason: SourceVersionSkip::SinkVersion { version: 2 }
        }));

        // Only on a scrambled link, to a sink reporting a non-zero version.
        let mut io = TestIo::new();
        io.sink_version = 3;
        run(
            &mut io,
            &scrambled,
            &only(SourceVersionWrite::WhenScrambling),
        );
        assert_eq!(io.ops(), [Op::ReadSinkVersion, Op::WriteSourceVersion(1)]);
        let mut io = TestIo::new();
        io.sink_version = 0;
        run(
            &mut io,
            &scrambled,
            &only(SourceVersionWrite::WhenScrambling),
        );
        assert_eq!(io.ops(), [Op::ReadSinkVersion]);
        let mut io = TestIo::new();
        let r = run(&mut io, &plain, &only(SourceVersionWrite::WhenScrambling));
        assert_eq!(io.ops().len(), 0);
        assert!(r.has(TmdsEventKind::SourceVersionSkipped {
            reason: SourceVersionSkip::NotScrambled
        }));

        // Never.
        let mut io = TestIo::new();
        let r = run(&mut io, &scrambled, &only(SourceVersionWrite::Never));
        assert_eq!(io.ops().len(), 0);
        assert!(r.has(TmdsEventKind::SourceVersionSkipped {
            reason: SourceVersionSkip::Never
        }));
    }

    #[test]
    fn transient_failures_are_warnings_and_the_work_goes_on() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        let r = run(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
        assert_eq!(r.result, Ok(()));
        assert_eq!(
            io.ops(),
            [
                Op::ClearFrlRate,
                Op::ClearFltUpdate,
                Op::WriteSourceVersion(1)
            ]
        );
        let w: [_; 3] = core::array::from_fn(|i| *r.warnings.iter_warnings().nth(i).unwrap());
        assert_eq!(
            w,
            [
                BringUpWarning::FrlRateClearFailed(DdcErrorKind::Busy),
                BringUpWarning::FltUpdateClearFailed(UpdateError::Read(DdcErrorKind::Busy)),
                BringUpWarning::SourceVersionWriteFailed(DdcErrorKind::Busy),
            ]
        );
        assert!(r.has(TmdsEventKind::ScdcStepFailed {
            step: TmdsStep::ClearFrlRate,
            kind: DdcErrorKind::Busy,
            ends: false
        }));

        // A failed Sink_Version read writes nothing.
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Timeout);
        let r = run(
            &mut io,
            &link(594_000, NO_FRL),
            &BringUpConfig::DEFAULT.with_source_version(SourceVersionWrite::IfSinkVersion1),
        );
        assert_eq!(io.ops(), [Op::ReadSinkVersion]);
        assert_eq!(
            r.warnings.iter_warnings().next(),
            Some(&BringUpWarning::SinkVersionReadFailed(
                DdcErrorKind::Timeout
            ))
        );
        assert!(r.has(TmdsEventKind::SourceVersionSkipped {
            reason: SourceVersionSkip::SinkVersionUnread
        }));
    }

    #[test]
    fn a_non_transient_error_is_a_warning_too() {
        for (ok_calls, step, warnings) in [
            (0, TmdsStep::ClearFrlRate, 3),
            (1, TmdsStep::ClearFltUpdate, 2),
            (2, TmdsStep::SourceVersion, 1),
        ] {
            let mut io = TestIo::new();
            io.fail_scdc = Some(DdcErrorKind::Detached);
            io.scdc_ok_calls = ok_calls;
            let r = run(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
            assert_eq!(r.result, Ok(()));
            // Every step is still attempted, and each failure is a warning.
            assert_eq!(io.ops().len(), 3);
            assert_eq!(r.warnings.len(), warnings);
            assert!(r.has(TmdsEventKind::ScdcStepFailed {
                step,
                kind: DdcErrorKind::Detached,
                ends: false
            }));
        }
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Unsupported);
        let r = run(
            &mut io,
            &link(594_000, NO_FRL),
            &BringUpConfig::DEFAULT.with_source_version(SourceVersionWrite::IfSinkVersion1),
        );
        assert_eq!(r.result, Ok(()));
        assert!(matches!(
            r.warnings.iter_warnings().next(),
            Some(BringUpWarning::SinkVersionReadFailed(
                DdcErrorKind::Unsupported
            ))
        ));
        assert!(r.has(TmdsEventKind::SourceVersionSkipped {
            reason: SourceVersionSkip::SinkVersionUnread
        }));
    }

    #[test]
    fn the_flag_is_read_before_each_step_before_both_paths() {
        for (after, step) in [
            (0, TmdsStep::ClearFrlRate),
            (1, TmdsStep::ClearFltUpdate),
            (2, TmdsStep::SourceVersion),
        ] {
            let mut io = TestIo::new();
            io.cancel_after_ops = Some(after);
            let r = run(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
            assert_eq!(r.result, Err(Halt::Cancelled(CancelPoint::Step(step))));
            assert_eq!(io.ops().len(), after);
        }
        let mut io = TestIo::new();
        io.cancelled = true;
        let config = BringUpConfig::DEFAULT.with_source_version(SourceVersionWrite::IfSinkVersion1);
        let r = run(&mut io, &link(594_000, NO_FRL), &config);
        assert_eq!(
            r.result,
            Err(Halt::Cancelled(CancelPoint::Step(TmdsStep::SinkVersion)))
        );
        // Skipped steps read nothing.
        let r = run(&mut io, &link(148_500, SinkTmdsFacts::NONE), &config);
        assert_eq!(r.result, Ok(()));
    }

    #[test]
    fn a_kind_added_later_counts_as_transient() {
        assert!(transient(DdcErrorKind::Busy));
        assert!(transient(DdcErrorKind::Other));
        assert!(!transient(DdcErrorKind::Detached));
        assert!(!transient(DdcErrorKind::Unsupported));
    }

    // ---- The whole bring-up ----

    use crate::config::{OutputDuringSinkWrite, TearDownConfig, TmdsConfigWrite};
    use crate::outcome::{SinkClear, TearDownError, TornDown};
    use crate::types::{SinkTmdsConfig, TmdsConfigGate};
    use crate::{CheckResult, ClockRatio, MAX_BRING_UP_EVENTS};
    use hdmi_hal::phy::TmdsVideo;

    const VIDEO: TmdsVideo = TmdsVideo::new(
        594_000,
        hdmi_hal::ColorBitDepth::Depth8,
        hdmi_hal::ColorFormat::Rgb444,
    );

    /// No FRL clear and no `Source_Version`: only the main path's SCDC calls.
    const MAIN_ONLY: BringUpConfig = BringUpConfig::DEFAULT
        .with_clear_frl(ClearFrl::IfSinkDeclaresFrl)
        .with_source_version(SourceVersionWrite::Never);

    type Outcome = Result<BringUpOutcome<DdcErrorKind, ()>, BringUpError<DdcErrorKind, ()>>;

    struct Up {
        result: Outcome,
        events: [Option<TmdsEventKind>; 32],
        count: usize,
    }

    impl Up {
        fn has(&self, kind: TmdsEventKind) -> bool {
            self.events[..self.count].contains(&Some(kind))
        }

        fn link_up(&self) -> Option<&LinkUp<DdcErrorKind>> {
            match &self.result {
                Ok(BringUpOutcome::Up(up)) => Some(up),
                _ => None,
            }
        }
    }

    fn up(io: &mut TestIo, link: &TmdsLinkParams, config: &BringUpConfig) -> Up {
        let mut events = [None; 32];
        let mut count = 0;
        let result = block_on(bring_up(io, link, VIDEO, config, &mut |e| {
            events[count] = Some(e.kind);
            count += 1;
        }));
        assert!(count <= MAX_BRING_UP_EVENTS, "{count} events");
        Up {
            result,
            events,
            count,
        }
    }

    fn phy(link: &TmdsLinkParams) -> Op {
        Op::Configure(TmdsPhyConfig::new(
            link.rate_khz(),
            link.clock_ratio(),
            VIDEO,
        ))
    }

    #[test]
    fn a_scrambled_link_comes_up_in_the_default_order() {
        let link = link(594_000, FRL_SINK);
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        let r = up(&mut io, &link, &BringUpConfig::DEFAULT);
        let ops = io.ops();
        assert_eq!(
            ops[..7],
            [
                Op::ClearFrlRate,
                Op::ClearFltUpdate,
                Op::WriteSourceVersion(1),
                Op::StopOutput,
                Op::UpdateTmdsConfig(SinkTmdsConfig::new(true, true)),
                phy(&link),
                Op::SetScrambling(true),
            ]
        );
        assert!(
            matches!(ops[7], Op::Delay(d) if d > Duration::ZERO && d <= Duration::from_millis(1))
        );
        assert_eq!(ops[8], Op::StartOutput);
        assert_eq!(ops[9], Op::ReadScramblingStatus);
        assert_eq!(ops.len(), 10);

        let up = r.link_up().unwrap();
        assert_eq!(up.link, link);
        assert!(up.warnings.is_empty());
        assert_eq!(up.check.result, CheckResult::Set);
        assert_eq!(up.check.reads, 1);
        let written = up.sink_written_at.unwrap();
        // The wait is measured from the write.
        assert!(
            up.output_started_at.saturating_duration_since(written) >= Duration::from_millis(1)
        );
        assert!(r.has(TmdsEventKind::PhyConfigured {
            character_rate_khz: 594_000,
            clock_ratio: ClockRatio::Div40
        }));
        assert!(r.has(TmdsEventKind::BringUpEnded {
            failed_at: None,
            cancelled: false
        }));
    }

    #[test]
    fn unscrambled_output_runs_through_the_write_with_running() {
        let link = link(594_000, NO_FRL);
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        let config = MAIN_ONLY.with_output_during_sink_write(OutputDuringSinkWrite::Running);
        let r = up(&mut io, &link, &config);
        let ops = io.ops();
        assert_eq!(
            ops[..5],
            [
                Op::StopOutput,
                phy(&link),
                Op::SetScrambling(false),
                Op::StartOutput,
                Op::UpdateTmdsConfig(SinkTmdsConfig::new(true, true)),
            ]
        );
        assert!(matches!(ops[5], Op::Delay(_)));
        assert_eq!(ops[6], Op::SetScrambling(true));
        assert!(r.has(TmdsEventKind::OutputStarted { after_write: None }));
        assert!(r.link_up().unwrap().warnings.is_empty());
    }

    #[test]
    fn the_check_runs_from_the_scrambler_with_running_and_not_on_a_plain_link() {
        let mut io = TestIo::new();
        io.status_set_from_read = Some(3);
        let config = MAIN_ONLY.with_output_during_sink_write(OutputDuringSinkWrite::Running);
        let r = up(&mut io, &link(594_000, NO_FRL), &config);
        let check = r.link_up().unwrap().check;
        assert_eq!((check.result, check.reads), (CheckResult::Set, 3));
        assert!(r.has(TmdsEventKind::ScramblingChecked { check }));

        let mut io = TestIo::new();
        let r = up(&mut io, &link(148_500, NO_FRL), &MAIN_ONLY);
        assert_eq!(
            r.link_up().unwrap().check.result,
            CheckResult::NotApplicable
        );
        assert!(!io.ops().contains(&Op::ReadScramblingStatus));
    }

    #[test]
    fn a_plain_link_clears_the_sink_and_has_no_deadline() {
        let link = link(148_500, NO_FRL);
        let mut io = TestIo::new();
        let config = MAIN_ONLY.with_scrambling_deadline(Duration::ZERO);
        let r = up(&mut io, &link, &config);
        assert_eq!(
            io.ops()[1],
            Op::UpdateTmdsConfig(SinkTmdsConfig::new(false, false))
        );
        assert_eq!(io.ops()[3], Op::SetScrambling(false));
        assert!(r.link_up().unwrap().warnings.is_empty());
    }

    #[test]
    fn whole_byte_writes_without_reading() {
        let mut io = TestIo::new();
        up(
            &mut io,
            &link(594_000, NO_FRL),
            &MAIN_ONLY.with_tmds_config_write(TmdsConfigWrite::WholeByte),
        );
        assert_eq!(
            io.ops()[1],
            Op::WriteTmdsConfig(SinkTmdsConfig::new(true, true))
        );
    }

    #[test]
    fn no_write_is_made_where_none_is_due() {
        let mut io = TestIo::new();
        let r = up(
            &mut io,
            &link(148_500, SinkTmdsFacts::NONE),
            &BringUpConfig::DEFAULT,
        );
        assert!(r.has(TmdsEventKind::TmdsConfigSkipped {
            reason: TmdsConfigSkip::NoScdc
        }));
        assert!(!io.ops().iter().any(|op| matches!(op, Op::Delay(_))));
        assert_eq!(r.link_up().unwrap().sink_written_at, None);

        let gated = TmdsLinkParams::choose(
            148_500,
            SinkTmdsFacts::new(true, false, false),
            SourceTmdsFacts::new(false),
            TmdsPolicy::DEFAULT.with_tmds_config_gate(TmdsConfigGate::ScramblingSink),
        )
        .unwrap();
        let mut io = TestIo::new();
        let r = up(&mut io, &gated, &MAIN_ONLY);
        assert!(r.has(TmdsEventKind::TmdsConfigSkipped {
            reason: TmdsConfigSkip::Gate
        }));
    }

    #[test]
    fn the_wait_runs_from_the_write_and_counts_what_came_after_it() {
        // The PHY's two calls after the write cost 300 µs each: of the 1 ms wait, a little
        // under 400 µs is left to spend.
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        io.op_micros = 300;
        let r = up(&mut io, &link(594_000, NO_FRL), &MAIN_ONLY);
        let delays: [Duration; 1] = {
            let mut found = [Duration::ZERO; 1];
            let mut n = 0;
            for op in io.ops() {
                if let Op::Delay(d) = op {
                    found[n] = *d;
                    n += 1;
                }
            }
            assert_eq!(n, 1);
            found
        };
        let left = delays[0];
        assert!(
            left > Duration::from_micros(390) && left < Duration::from_micros(400),
            "{left:?}"
        );
        assert!(r.has(TmdsEventKind::WaitSpent { duration: left }));

        // Calls that alone take longer than the wait leave nothing to spend.
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        io.op_micros = 600;
        let r = up(&mut io, &link(594_000, NO_FRL), &MAIN_ONLY);
        assert!(!io.ops().iter().any(|op| matches!(op, Op::Delay(_))));
        assert!(r.has(TmdsEventKind::WaitSpent {
            duration: Duration::ZERO
        }));
    }

    #[test]
    fn a_read_modify_write_that_fails_at_the_write_says_so() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.update_fails_at_write = true;
        let r = up(&mut io, &link(594_000, NO_FRL), &MAIN_ONLY);
        assert!(matches!(
            r.link_up().unwrap().warnings.iter_warnings().next(),
            Some(BringUpWarning::TmdsConfigWriteFailed {
                error: UpdateError::Write(DdcErrorKind::Busy),
                ..
            })
        ));
        assert!(r.has(TmdsEventKind::ScdcStepFailed {
            step: TmdsStep::WriteTmdsConfig,
            kind: DdcErrorKind::Busy,
            ends: false
        }));
    }

    #[test]
    fn a_check_that_sees_no_scrambling_is_a_warning() {
        let link = link(594_000, NO_FRL);
        // Set: no warning.
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        assert!(
            up(&mut io, &link, &MAIN_ONLY)
                .link_up()
                .unwrap()
                .warnings
                .is_empty()
        );
        // Not set within the window, and unreadable.
        for (fail, result) in [
            (None, CheckResult::NotSet),
            (Some(DdcErrorKind::Busy), CheckResult::Unreadable),
        ] {
            let mut io = TestIo::new();
            io.fail_scdc = fail;
            io.scdc_ok_calls = 1; // the sink's write succeeds
            let r = up(&mut io, &link, &MAIN_ONLY);
            let up = r.link_up().unwrap();
            assert_eq!(up.check.result, result);
            assert!(
                up.warnings
                    .iter_warnings()
                    .any(|w| *w == BringUpWarning::ScramblingNotConfirmed { result }),
                "{result:?}"
            );
        }
        // A plain link has no check and no warning.
        let mut io = TestIo::new();
        let r = up(&mut io, &super::tests::link(148_500, NO_FRL), &MAIN_ONLY);
        assert!(r.link_up().unwrap().warnings.is_empty());
    }

    #[test]
    fn a_late_start_is_a_warning() {
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        let config = MAIN_ONLY.with_scrambling_deadline(Duration::from_micros(500));
        let r = up(&mut io, &link(594_000, NO_FRL), &config);
        let w = r
            .link_up()
            .unwrap()
            .warnings
            .iter_warnings()
            .next()
            .copied();
        assert!(
            matches!(w, Some(BringUpWarning::DeadlineMissed { by }) if by >= Duration::from_micros(500))
        );
        assert!(
            r.events[..r.count]
                .iter()
                .any(|k| matches!(k, Some(TmdsEventKind::DeadlineMissed { .. })))
        );

        // With Running, the deadline runs from the write to the scrambler.
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        let r = up(
            &mut io,
            &link(594_000, NO_FRL),
            &config.with_output_during_sink_write(OutputDuringSinkWrite::Running),
        );
        assert_eq!(r.link_up().unwrap().warnings.len(), 1);
    }

    #[test]
    fn a_failed_write_carries_on_and_a_plain_link_reads_back() {
        // A scrambled link: the check covers it, nothing read back.
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.scdc_failures = 1;
        io.scrambling_status = 0x01;
        let r = up(&mut io, &link(594_000, NO_FRL), &MAIN_ONLY);
        assert_eq!(
            r.link_up().unwrap().warnings.iter_warnings().next(),
            Some(&BringUpWarning::TmdsConfigWriteFailed {
                error: UpdateError::Read(DdcErrorKind::Busy),
                read_back: None
            })
        );
        // The read half failed, and the event says so by its step.
        assert!(r.has(TmdsEventKind::ScdcStepFailed {
            step: TmdsStep::ReadTmdsConfig,
            kind: DdcErrorKind::Busy,
            ends: false
        }));
        // It carried on to the output and the check. Nothing was written, so there is no
        // write time, no wait and no deadline to run from one.
        assert_eq!(
            io.ops()[io.ops().len() - 2..],
            [Op::StartOutput, Op::ReadScramblingStatus]
        );
        assert_eq!(r.link_up().unwrap().sink_written_at, None);
        assert!(!io.ops().iter().any(|op| matches!(op, Op::Delay(_))));
        assert!(r.has(TmdsEventKind::OutputStarted { after_write: None }));

        // A plain link: the register read back after the failure.
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.scdc_failures = 1;
        io.tmds_config = 0x03;
        let r = up(
            &mut io,
            &link(148_500, NO_FRL),
            &MAIN_ONLY.with_tmds_config_write(TmdsConfigWrite::WholeByte),
        );
        let read_back = r
            .link_up()
            .unwrap()
            .warnings
            .iter_warnings()
            .next()
            .copied();
        assert!(matches!(
            read_back,
            Some(BringUpWarning::TmdsConfigWriteFailed {
                error: UpdateError::Write(DdcErrorKind::Busy),
                read_back: Some(Ok(read)),
            }) if read.config == SinkTmdsConfig::new(true, true) && read.raw == 0x03
        ));
    }

    #[test]
    fn a_failed_write_stops_under_stop_and_tears_down() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        let config = MAIN_ONLY.with_on_tmds_config_failure(OnTmdsConfigFailure::Stop);
        let r = up(&mut io, &link(594_000, NO_FRL), &config);
        assert_eq!(
            r.result,
            // The read half of the read-modify-write failed: nothing was written.
            Err(BringUpError::Scdc {
                step: TmdsStep::ReadTmdsConfig,
                error: DdcErrorKind::Busy,
                output: OutputState::Stopped,
                cleanup: Cleanup::TornDown(Ok(TornDown {
                    sink: SinkClear::NotRequested
                })),
                warnings: Warnings::new(),
            })
        );
        assert_eq!(io.ops().last(), Some(&Op::StopOutput));
        assert!(r.has(TmdsEventKind::BringUpEnded {
            failed_at: Some(TmdsStep::ReadTmdsConfig),
            cancelled: false
        }));
        // The write half names the write.
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.update_fails_at_write = true;
        let r = up(&mut io, &link(594_000, NO_FRL), &config);
        assert!(matches!(
            r.result,
            Err(BringUpError::Scdc {
                step: TmdsStep::WriteTmdsConfig,
                ..
            })
        ));
    }

    #[test]
    fn a_gone_sink_gets_no_writes_in_the_cleanup() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Detached);
        let config = MAIN_ONLY.with_tear_down(TearDownConfig::DEFAULT.with_clear_sink(true));
        let r = up(&mut io, &link(594_000, NO_FRL), &config);
        assert!(matches!(
            r.result,
            Err(BringUpError::Scdc {
                cleanup: Cleanup::TornDown(Ok(TornDown {
                    sink: SinkClear::SkippedSinkGone
                })),
                ..
            })
        ));
    }

    #[test]
    fn a_phy_failure_says_where_and_whether_output_may_run() {
        for (fail, step, output) in [
            (0, TmdsStep::StopOutput, OutputState::MayBeRunning),
            (1, TmdsStep::Configure, OutputState::Stopped),
            (2, TmdsStep::SetScrambling, OutputState::Stopped),
            (3, TmdsStep::StartOutput, OutputState::MayBeRunning),
        ] {
            let mut io = TestIo::new();
            match fail {
                0 => io.fail_stop_output = true,
                1 => io.fail_configure = true,
                2 => io.fail_set_scrambling = true,
                _ => io.fail_start_output = true,
            }
            let r = up(&mut io, &link(594_000, NO_FRL), &MAIN_ONLY);
            assert!(
                matches!(
                    r.result,
                    Err(BringUpError::Phy {
                        step: s,
                        output: o,
                        cleanup: Cleanup::TornDown(_),
                        ..
                    }) if (s, o) == (step, output)
                ),
                "{step:?}"
            );
        }
        // The cleanup's own failure is in it.
        let mut io = TestIo::new();
        io.fail_stop_output = true;
        let r = up(&mut io, &link(594_000, NO_FRL), &MAIN_ONLY);
        assert!(matches!(
            r.result,
            Err(BringUpError::Phy {
                cleanup: Cleanup::TornDown(Err(TearDownError {
                    output: Some(()),
                    ..
                })),
                ..
            })
        ));
    }

    #[test]
    fn without_tear_down_on_error_both_ends_stay() {
        let mut io = TestIo::new();
        io.fail_start_output = true;
        let r = up(
            &mut io,
            &link(594_000, NO_FRL),
            &MAIN_ONLY.with_tear_down_on_error(false),
        );
        assert!(matches!(
            r.result,
            Err(BringUpError::Phy {
                cleanup: Cleanup::Skipped,
                ..
            })
        ));
        assert_eq!(io.ops().last(), Some(&Op::StartOutput));
    }

    #[test]
    fn warnings_before_both_paths_are_kept_until_the_write_stops() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.scdc_failures = 1; // the FRL rate clear: a warning
        io.scdc_ok_calls = 0;
        let mut config = BringUpConfig::DEFAULT;
        config.on_tmds_config_failure = OnTmdsConfigFailure::Stop;
        let r = up(&mut io, &link(594_000, FRL_SINK), &config);
        assert!(
            r.link_up()
                .unwrap()
                .warnings
                .iter_warnings()
                .any(|w| matches!(w, BringUpWarning::FrlRateClearFailed(_)))
        );

        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Unsupported);
        let r = up(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
        // The work before both paths only warns; the sink's write stops the bring-up.
        assert!(matches!(
            r.result,
            Err(BringUpError::Scdc {
                step: TmdsStep::ReadTmdsConfig,
                output: OutputState::Stopped,
                warnings,
                ..
            }) if warnings.len() == 3
        ));
    }

    /// Where the bring-up stopped and what it left: `None` when the link came up.
    fn cancelled(r: &Up) -> Option<(CancelPoint, &Cleanup<DdcErrorKind, ()>)> {
        match &r.result {
            Ok(BringUpOutcome::Cancelled { at, cleanup, .. }) => Some((*at, cleanup)),
            _ => None,
        }
    }

    #[test]
    fn a_flag_set_at_entry_ends_it_before_any_io() {
        let mut io = TestIo::new();
        io.cancelled = true;
        let r = up(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
        assert_eq!(
            r.result,
            Ok(BringUpOutcome::Cancelled {
                at: CancelPoint::Entry,
                cleanup: Cleanup::Skipped,
                warnings: Warnings::new(),
            })
        );
        assert!(io.ops().is_empty());
        assert_eq!(r.count, 3);
        assert_eq!(
            r.events[1],
            Some(TmdsEventKind::BringUpCancelled {
                at: CancelPoint::Entry
            })
        );
        assert_eq!(
            r.events[2],
            Some(TmdsEventKind::BringUpEnded {
                failed_at: None,
                cancelled: true
            })
        );
    }

    #[test]
    fn a_cancel_is_seen_at_the_next_point_and_tears_down() {
        // Ops of the full run: 0 ClearFrlRate, 1 ClearFltUpdate, 2 WriteSourceVersion,
        // 3 StopOutput, 4 UpdateTmdsConfig, 5 Configure, 6 SetScrambling, 7-9 the wait's
        // three slices, 10 StartOutput, then the check: 11 a read, 12 a delay, 13 a read,
        // 14 a delay, 15 the read that sees scrambling.
        let config = BringUpConfig::DEFAULT.with_ratio_wait(Duration::from_millis(3));
        let link = link(594_000, FRL_SINK);
        let run = |after: Option<usize>| {
            let mut io = TestIo::new();
            io.status_set_from_read = Some(3);
            io.cancel_after_ops = after;
            let r = up(&mut io, &link, &config);
            (r, io)
        };
        let (full, io) = run(None);
        assert!(full.link_up().is_some());
        assert_eq!(io.ops().len(), 16);
        let mut all = [Op::StopOutput; 16];
        all.copy_from_slice(io.ops());
        assert!(
            all[7..10]
                .iter()
                .all(|op| matches!(op, Op::Delay(d) if *d <= Duration::from_millis(1)))
        );

        let step = CancelPoint::Step;
        for (after, at, ran) in [
            (1, step(TmdsStep::ClearFltUpdate), 1),
            (2, step(TmdsStep::SourceVersion), 2),
            // The PHY's calls are not points: the next one is the sink's write.
            (3, step(TmdsStep::WriteTmdsConfig), 4),
            (4, step(TmdsStep::WriteTmdsConfig), 4),
            (5, CancelPoint::RatioWait, 7),
            (8, CancelPoint::RatioWait, 8),
            (10, CancelPoint::RatioWait, 10),
            (11, CancelPoint::Check, 11),
            (12, CancelPoint::Check, 13),
            (15, CancelPoint::Check, 15),
        ] {
            let (r, io) = run(Some(after));
            let (seen, cleanup) = cancelled(&r).expect("cancelled");
            assert_eq!(seen, at, "after {after}");
            assert_eq!(io.ops()[..ran], all[..ran], "after {after}");
            // Then the tear-down, whatever `tear_down_on_error` says: the output stopped.
            assert_eq!(io.ops()[ran..], [Op::StopOutput], "after {after}");
            assert_eq!(
                cleanup,
                &Cleanup::TornDown(Ok(TornDown {
                    sink: SinkClear::NotRequested
                }))
            );
            assert!(r.has(TmdsEventKind::BringUpCancelled { at }));
            assert!(r.has(TmdsEventKind::BringUpEnded {
                failed_at: None,
                cancelled: true
            }));
        }
        // A cancelled check is recorded, and it ends the bring-up rather than its outcome.
        let (r, _) = run(Some(11));
        assert!(r.events[..r.count].iter().any(|e| matches!(
            e,
            Some(TmdsEventKind::ScramblingChecked { check }) if check.result == CheckResult::Cancelled
        )));
        // Once the check saw scrambling, a later flag changes nothing.
        let (r, _) = run(Some(16));
        assert!(cancelled(&r).is_none());
        let mut io = TestIo::new();
        io.cancel_after_ops = Some(3);
        let r = up(&mut io, &link, &config.with_tear_down_on_error(false));
        assert!(matches!(cancelled(&r), Some((_, Cleanup::TornDown(_)))));
    }

    #[test]
    fn a_plain_link_reads_the_flag_before_its_read_back() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.scdc_failures = 1;
        io.cancel_after_ops = Some(2); // StopOutput, then the failed write
        let r = up(&mut io, &link(148_500, NO_FRL), &MAIN_ONLY);
        assert!(matches!(
            &r.result,
            Ok(BringUpOutcome::Cancelled {
                at: CancelPoint::Step(TmdsStep::ReadTmdsConfig),
                warnings,
                ..
            }) if warnings.iter_warnings().any(|w| matches!(
                w,
                BringUpWarning::TmdsConfigWriteFailed { read_back: None, .. }
            ))
        ));
        assert!(!io.ops().contains(&Op::ReadTmdsConfig));
    }

    #[test]
    fn a_zero_interval_spends_the_wait_in_one_delay() {
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        let config = MAIN_ONLY
            .with_ratio_wait(Duration::from_millis(3))
            .with_check_interval(Duration::ZERO);
        up(&mut io, &link(594_000, NO_FRL), &config);
        let delays = io
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Delay(_)))
            .count();
        assert_eq!(delays, 1);
    }

    #[test]
    fn no_combination_records_more_than_the_bound() {
        for rate in [148_500, 594_000] {
            for sink in [FRL_SINK, NO_FRL, SinkTmdsFacts::NONE] {
                for running in [false, true] {
                    for whole in [false, true] {
                        for fail in 0..11 {
                            let Ok(link) = TmdsLinkParams::choose(
                                rate,
                                sink,
                                SourceTmdsFacts::new(false),
                                TmdsPolicy::DEFAULT,
                            ) else {
                                continue; // a rate the sink cannot take
                            };
                            let mut io = TestIo::new();
                            match fail {
                                1 => io.fail_scdc = Some(DdcErrorKind::Busy),
                                2 => io.fail_scdc = Some(DdcErrorKind::Detached),
                                3 => io.fail_configure = true,
                                4 => io.fail_set_scrambling = true,
                                5 => io.fail_start_output = true,
                                6 => io.fail_stop_output = true,
                                7 => {
                                    io.fail_scdc = Some(DdcErrorKind::Busy);
                                    io.fail_stop_output = true;
                                }
                                8 => io.cancel_after_ops = Some(4),
                                9 => io.cancel_after_ops = Some(8),
                                10 => {
                                    io.fail_scdc = Some(DdcErrorKind::Busy);
                                    io.cancel_after_ops = Some(5);
                                }
                                _ => {}
                            }
                            let mut config = BringUpConfig::DEFAULT
                                .with_clear_frl(ClearFrl::EveryScdcSink)
                                .with_source_version(SourceVersionWrite::IfSinkVersion1)
                                .with_scrambling_deadline(Duration::ZERO)
                                .with_tear_down(TearDownConfig::DEFAULT.with_clear_sink(true));
                            if running {
                                config.output_during_sink_write = OutputDuringSinkWrite::Running;
                            }
                            if whole {
                                config.tmds_config_write = TmdsConfigWrite::WholeByte;
                            }
                            let r = up(&mut io, &link, &config);
                            let cancelled =
                                matches!(r.result, Ok(BringUpOutcome::Cancelled { .. }));
                            assert_eq!(r.link_up().is_some(), r.result.is_ok() && !cancelled);
                        }
                    }
                }
            }
        }
    }
}
