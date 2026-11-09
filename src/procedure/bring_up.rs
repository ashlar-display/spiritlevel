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
use crate::event::{
    FrlClearSkip, SourceVersionSkip, TmdsConfigSkip, TmdsEvent, TmdsEventKind, TmdsStep,
};
use crate::outcome::{
    BringUpError, BringUpOutcome, BringUpWarning, Cleanup, LinkUp, OutputState, ScramblingCheck,
    Warnings, WriteStage,
};
use crate::params::TmdsLinkParams;

use super::{TmdsIo, tear_down};

/// A step that stopped the bring-up, and its error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stop<S> {
    pub(crate) step: TmdsStep,
    pub(crate) error: S,
}

/// Whether an error may pass: everything but a bus that is gone (`Detached`) or a request
/// that cannot be done (`Unsupported`). A kind added later counts as transient.
fn transient(kind: DdcErrorKind) -> bool {
    !matches!(kind, DdcErrorKind::Detached | DdcErrorKind::Unsupported)
}

/// A failed SCDC step: a warning to carry on with, or the stop.
fn failed<Io: TmdsIo>(
    io: &Io,
    emit: &mut impl FnMut(&Io, TmdsEventKind),
    step: TmdsStep,
    error: Io::ScdcError,
    warning: impl FnOnce(Io::ScdcError) -> BringUpWarning<Io::ScdcError>,
    warnings: &mut Warnings<Io::ScdcError>,
) -> Result<(), Stop<Io::ScdcError>> {
    let kind = error.kind();
    let ends = !transient(kind);
    emit(io, TmdsEventKind::ScdcStepFailed { step, kind, ends });
    if ends {
        Err(Stop { step, error })
    } else {
        warnings.push(warning(error));
        Ok(())
    }
}

/// The work before both paths, in order: the FRL clear (`Config_1`, then `FLT_update`) when
/// the settings give it, then `Source_Version`. Nothing on a sink without SCDC. A failed
/// write is a warning; a non-transient error stops the bring-up.
pub(crate) async fn before_both_paths<Io: TmdsIo>(
    io: &mut Io,
    link: &TmdsLinkParams,
    config: &BringUpConfig,
    warnings: &mut Warnings<Io::ScdcError>,
    emit: &mut impl FnMut(&Io, TmdsEventKind),
) -> Result<(), Stop<Io::ScdcError>> {
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
            match io.clear_frl_rate().await {
                Ok(()) => emit(io, TmdsEventKind::FrlRateCleared),
                Err(e) => failed(
                    io,
                    emit,
                    TmdsStep::ClearFrlRate,
                    e,
                    BringUpWarning::FrlRateClearFailed,
                    warnings,
                )?,
            }
            match io.clear_flt_update().await {
                Ok(was_set) => emit(io, TmdsEventKind::FltUpdateCleared { was_set }),
                Err(e) => failed(
                    io,
                    emit,
                    TmdsStep::ClearFltUpdate,
                    e.into_inner(),
                    BringUpWarning::FltUpdateClearFailed,
                    warnings,
                )?,
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
                )?;
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
    match io.write_source_version(1).await {
        Ok(()) => emit(io, TmdsEventKind::SourceVersionWritten { version: 1 }),
        Err(e) => failed(
            io,
            emit,
            TmdsStep::SourceVersion,
            e,
            BringUpWarning::SourceVersionWriteFailed,
            warnings,
        )?,
    }
    Ok(())
}

/// Why a bring-up stopped, before the cleanup: a step, its error, and the output's state.
enum Failure<S, P> {
    Scdc(Stop<S>, OutputState),
    Phy(TmdsStep, P, OutputState),
}

/// What the main path produced, for `LinkUp`.
struct Up {
    sink_written_at: Option<Timestamp>,
    output_started_at: Timestamp,
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
/// none was due), or the stop. A transient failure under `CarryOn` is a warning; on a plain
/// link it reads the register back.
async fn write_sink<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    link: &TmdsLinkParams,
    config: &BringUpConfig,
    warnings: &mut Warnings<Io::ScdcError>,
    record: &mut F,
) -> Result<Option<Timestamp>, Stop<Io::ScdcError>> {
    let Some(value) = link.sink_config() else {
        let reason = if link.sink().scdc_present {
            TmdsConfigSkip::Gate
        } else {
            TmdsConfigSkip::NoScdc
        };
        emit(io, record, TmdsEventKind::TmdsConfigSkipped { reason });
        return Ok(None);
    };
    let result = match config.tmds_config_write {
        TmdsConfigWrite::WholeByte => io
            .write_tmds_config(value)
            .await
            .map_err(|e| (WriteStage::Write, e)),
        _ => io.update_tmds_config(value).await.map_err(|e| match e {
            crate::error::UpdateError::Read(e) => (WriteStage::Read, e),
            crate::error::UpdateError::Write(e) => (WriteStage::Write, e),
        }),
    };
    let written_at = io.now();
    let (stage, error) = match result {
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
    let step = TmdsStep::WriteTmdsConfig;
    let kind = error.kind();
    let ends = !transient(kind) || config.on_tmds_config_failure == OnTmdsConfigFailure::Stop;
    emit(
        io,
        record,
        TmdsEventKind::ScdcStepFailed { step, kind, ends },
    );
    if ends {
        return Err(Stop { step, error });
    }
    // A plain link has no scrambling check: read the register back so a sink that kept
    // 1/40 after a failed clear does not go unnoticed.
    let read_back = if link.scrambling() {
        None
    } else {
        let read = io.read_tmds_config().await;
        match &read {
            Ok(config) => emit(
                io,
                record,
                TmdsEventKind::TmdsConfigRead { config: *config },
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
    warnings.push(BringUpWarning::TmdsConfigWriteFailed {
        error,
        stage,
        read_back,
    });
    Ok(Some(written_at))
}

/// Spends what is left of the ratio wait after the write at `written_at`; nothing when no
/// write was made.
async fn wait_after<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    written_at: Option<Timestamp>,
    wait: Duration,
    record: &mut F,
) {
    let Some(written_at) = written_at else {
        return;
    };
    let left = wait.saturating_sub(io.now().saturating_duration_since(written_at));
    if !left.is_zero() {
        io.delay(left).await;
    }
    emit(io, record, TmdsEventKind::WaitSpent { duration: left });
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
) -> Result<Up, Failure<Io::ScdcError, Io::PhyError>> {
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
            .map_err(|stop| Failure::Scdc(stop, OutputState::Stopped))?;
        configure(io, phy, record).await?;
        set_scrambling(io, scrambled, OutputState::Stopped, record).await?;
        wait_after(io, written_at, config.ratio_wait, record).await;
        let started = start(io, written_at, record).await?;
        deadline(io, written_at, started, scrambled, config, warnings, record);
        return Ok(Up {
            sink_written_at: written_at,
            output_started_at: started,
        });
    }

    // `Running`: unscrambled output first, then the sink's write, the wait, and only then
    // the source's scrambler, so no scrambled TMDS reaches the sink before it is told.
    configure(io, phy, record).await?;
    set_scrambling(io, false, OutputState::Stopped, record).await?;
    let started = start(io, None, record).await?;
    let written_at = write_sink(io, link, config, warnings, record)
        .await
        .map_err(|stop| Failure::Scdc(stop, OutputState::MayBeRunning))?;
    wait_after(io, written_at, config.ratio_wait, record).await;
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
    }
    Ok(Up {
        sink_written_at: written_at,
        output_started_at: started,
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
/// Records at most [`MAX_BRING_UP_EVENTS`](crate::MAX_BRING_UP_EVENTS) events.
///
/// # Errors
///
/// [`BringUpError`] when a PHY call failed, a sink write or read failed with a
/// non-transient error, or a `TMDS_Config` write failed under `OnTmdsConfigFailure::Stop`.
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
    let phy = TmdsPhyConfig::new(link.rate_khz(), link.clock_ratio(), video);

    let result = match before_both_paths(io, link, config, &mut warnings, &mut |io, kind| {
        emit(io, record, kind);
    })
    .await
    {
        Ok(()) => main_path(io, link, phy, config, &mut warnings, record).await,
        Err(stop) => Err(Failure::Scdc(stop, OutputState::MayBeRunning)),
    };

    let failure = match result {
        Ok(up) => {
            emit(io, record, TmdsEventKind::BringUpEnded { failed_at: None });
            return Ok(BringUpOutcome::Up(LinkUp {
                link: *link,
                sink_written_at: up.sink_written_at,
                output_started_at: up.output_started_at,
                check: ScramblingCheck::NOT_APPLICABLE,
                warnings,
            }));
        }
        Err(failure) => failure,
    };

    let (step, presence) = match &failure {
        Failure::Scdc(stop, _) if stop.error.kind() == DdcErrorKind::Detached => {
            (stop.step, SinkPresence::Gone)
        }
        Failure::Scdc(stop, _) => (stop.step, SinkPresence::Present),
        Failure::Phy(step, ..) => (*step, SinkPresence::Present),
    };
    let cleanup = if config.tear_down_on_error {
        Cleanup::TornDown(tear_down(io, link, &config.tear_down, presence, record).await)
    } else {
        Cleanup::Skipped
    };
    emit(
        io,
        record,
        TmdsEventKind::BringUpEnded {
            failed_at: Some(step),
        },
    );
    Err(match failure {
        Failure::Scdc(Stop { step, error }, output) => BringUpError::Scdc {
            step,
            error,
            output,
            cleanup,
            warnings,
        },
        Failure::Phy(step, error, output) => BringUpError::Phy {
            step,
            error,
            output,
            cleanup,
            warnings,
        },
    })
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
        result: Result<(), Stop<DdcErrorKind>>,
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
        let w: [_; 3] = core::array::from_fn(|i| *r.warnings.iter().nth(i).unwrap());
        assert_eq!(
            w,
            [
                BringUpWarning::FrlRateClearFailed(DdcErrorKind::Busy),
                BringUpWarning::FltUpdateClearFailed(DdcErrorKind::Busy),
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
            r.warnings.iter().next(),
            Some(&BringUpWarning::SinkVersionReadFailed(
                DdcErrorKind::Timeout
            ))
        );
        assert!(r.has(TmdsEventKind::SourceVersionSkipped {
            reason: SourceVersionSkip::SinkVersionUnread
        }));
    }

    #[test]
    fn a_non_transient_error_stops_at_its_step() {
        for (ok_calls, step) in [
            (0, TmdsStep::ClearFrlRate),
            (1, TmdsStep::ClearFltUpdate),
            (2, TmdsStep::SourceVersion),
        ] {
            let mut io = TestIo::new();
            io.fail_scdc = Some(DdcErrorKind::Detached);
            io.scdc_ok_calls = ok_calls;
            let r = run(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
            assert_eq!(
                r.result,
                Err(Stop {
                    step,
                    error: DdcErrorKind::Detached
                })
            );
            assert_eq!(io.ops().len(), ok_calls + 1);
            assert!(r.has(TmdsEventKind::ScdcStepFailed {
                step,
                kind: DdcErrorKind::Detached,
                ends: true
            }));
        }
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Unsupported);
        let r = run(
            &mut io,
            &link(594_000, NO_FRL),
            &BringUpConfig::DEFAULT.with_source_version(SourceVersionWrite::IfSinkVersion1),
        );
        assert_eq!(
            r.result,
            Err(Stop {
                step: TmdsStep::SinkVersion,
                error: DdcErrorKind::Unsupported
            })
        );
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
        assert_eq!(ops.len(), 9);

        let up = r.link_up().unwrap();
        assert_eq!(up.link, link);
        assert!(up.warnings.is_empty());
        assert_eq!(up.check.result, CheckResult::NotApplicable);
        let written = up.sink_written_at.unwrap();
        // The wait is measured from the write.
        assert!(
            up.output_started_at.saturating_duration_since(written) >= Duration::from_millis(1)
        );
        assert!(r.has(TmdsEventKind::PhyConfigured {
            character_rate_khz: 594_000,
            clock_ratio: ClockRatio::Div40
        }));
        assert!(r.has(TmdsEventKind::BringUpEnded { failed_at: None }));
    }

    #[test]
    fn unscrambled_output_runs_through_the_write_with_running() {
        let link = link(594_000, NO_FRL);
        let mut io = TestIo::new();
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
    fn no_delay_is_spent_once_the_wait_is_over() {
        let mut io = TestIo::new();
        let r = up(
            &mut io,
            &link(594_000, NO_FRL),
            &MAIN_ONLY.with_ratio_wait(Duration::ZERO),
        );
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
            r.link_up().unwrap().warnings.iter().next(),
            Some(BringUpWarning::TmdsConfigWriteFailed {
                stage: WriteStage::Write,
                ..
            })
        ));
    }

    #[test]
    fn a_late_start_is_a_warning() {
        let mut io = TestIo::new();
        let config = MAIN_ONLY.with_scrambling_deadline(Duration::from_micros(500));
        let r = up(&mut io, &link(594_000, NO_FRL), &config);
        let w = r.link_up().unwrap().warnings.iter().next().copied();
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
        let r = up(&mut io, &link(594_000, NO_FRL), &MAIN_ONLY);
        assert_eq!(
            r.link_up().unwrap().warnings.iter().next(),
            Some(&BringUpWarning::TmdsConfigWriteFailed {
                error: DdcErrorKind::Busy,
                stage: WriteStage::Read,
                read_back: None
            })
        );
        assert_eq!(io.ops().last(), Some(&Op::StartOutput));

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
        let read_back = r.link_up().unwrap().warnings.iter().next().copied();
        assert!(matches!(
            read_back,
            Some(BringUpWarning::TmdsConfigWriteFailed {
                stage: WriteStage::Write,
                read_back: Some(Ok(read)),
                ..
            }) if read == SinkTmdsConfig::new(true, true) && read.raw() == Some(0x03)
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
            Err(BringUpError::Scdc {
                step: TmdsStep::WriteTmdsConfig,
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
            failed_at: Some(TmdsStep::WriteTmdsConfig)
        }));
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
    fn an_early_stop_keeps_the_warnings_and_leaves_the_output_unknown() {
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
                .iter()
                .any(|w| matches!(w, BringUpWarning::FrlRateClearFailed(_)))
        );

        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Unsupported);
        let r = up(&mut io, &link(594_000, FRL_SINK), &BringUpConfig::DEFAULT);
        assert!(matches!(
            r.result,
            Err(BringUpError::Scdc {
                step: TmdsStep::ClearFrlRate,
                output: OutputState::MayBeRunning,
                ..
            })
        ));
    }

    #[test]
    fn no_combination_records_more_than_the_bound() {
        for rate in [148_500, 594_000] {
            for sink in [FRL_SINK, NO_FRL, SinkTmdsFacts::NONE] {
                for running in [false, true] {
                    for whole in [false, true] {
                        for fail in 0..8 {
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
                            assert_eq!(r.link_up().is_some(), r.result.is_ok());
                        }
                    }
                }
            }
        }
    }
}
