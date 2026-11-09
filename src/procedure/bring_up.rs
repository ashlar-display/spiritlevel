//! Bring-up: the stages a TMDS link is brought up in. This part: the work before both paths
//! (the FRL clear and `Source_Version`), on a sink with SCDC.

use hdmi_hal::ddc::{DdcError, DdcErrorKind};

use crate::config::{BringUpConfig, ClearFrl, SourceVersionWrite};
use crate::event::{FrlClearSkip, SourceVersionSkip, TmdsEventKind, TmdsStep};
use crate::outcome::{BringUpWarning, Warnings};
use crate::params::TmdsLinkParams;

use super::TmdsIo;

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
#[allow(dead_code, reason = "used by the bring-up procedure, which follows")]
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
}
