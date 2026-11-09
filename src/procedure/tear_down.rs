//! Tear-down: stop the TMDS link a bring-up started.

use hdmi_hal::ddc::DdcError;

use crate::config::{ScramblerOff, SinkClearTiming, SinkPresence, TearDownConfig};
use crate::event::{SinkClearSkip, TmdsEvent, TmdsEventKind, TmdsStep};
use crate::outcome::{SinkClear, TearDownError, TornDown};
use crate::params::TmdsLinkParams;
use crate::types::SinkTmdsConfig;

use super::TmdsIo;

/// The parts of a tear-down, in the order they run.
#[derive(Clone, Copy)]
enum Part {
    Sink,
    Scrambler,
    Output,
}

/// The order of the parts with clearing on, from the two settings: the sink before or after
/// the output stops, the scrambler before or after the sink.
const fn order(config: &TearDownConfig) -> [Part; 3] {
    match (config.sink_clear_timing, config.scrambler_off) {
        (SinkClearTiming::BeforeOutputStops, ScramblerOff::AfterSink) => {
            [Part::Sink, Part::Scrambler, Part::Output]
        }
        (SinkClearTiming::BeforeOutputStops, ScramblerOff::BeforeSink) => {
            [Part::Scrambler, Part::Sink, Part::Output]
        }
        (SinkClearTiming::AfterOutputStops, ScramblerOff::AfterSink) => {
            [Part::Output, Part::Sink, Part::Scrambler]
        }
        (SinkClearTiming::AfterOutputStops, ScramblerOff::BeforeSink) => {
            [Part::Scrambler, Part::Output, Part::Sink]
        }
    }
}

/// Stops the TMDS link `link` describes.
///
/// With `clear_sink` off (the default), only the output stops: the next bring-up writes the
/// sink's `TMDS_Config` and the source's scrambler again. With it on, the sink's
/// `TMDS_Config` is written to 0 and the scrambler turned off, in the order the settings give,
/// and the output stops. The sink is written only when the link set something (not on a
/// plain link or a sink without SCDC) and the caller says it is present.
///
/// Every step is attempted whatever failed before: a failed sink write is a warning
/// ([`SinkClear::Failed`]), and a failed PHY call still lets the output stop. Not cancelled:
/// tear-down is the recovery. Powering the PHY down is the caller's, not part of it.
///
/// Records at most [`MAX_TEAR_DOWN_EVENTS`](crate::MAX_TEAR_DOWN_EVENTS) events.
///
/// # Errors
///
/// [`TearDownError`] when a PHY step failed, with the first error of each failed step.
pub async fn tear_down<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    link: &TmdsLinkParams,
    config: &TearDownConfig,
    presence: SinkPresence,
    record: &mut F,
) -> Result<TornDown<Io::ScdcError>, TearDownError<Io::ScdcError, Io::PhyError>> {
    let mut emit = |io: &Io, kind| record(TmdsEvent::new(io.now(), kind));
    emit(
        io,
        TmdsEventKind::TearDownStarted {
            link: *link,
            config: *config,
            presence,
        },
    );

    let set_by_link = link
        .sink_config()
        .is_some_and(|c| c.scrambling_enable || c.high_tmds_clock_ratio);
    let mut skip = if !config.clear_sink {
        Some((SinkClearSkip::NotRequested, SinkClear::NotRequested))
    } else if !set_by_link {
        Some((SinkClearSkip::NothingToClear, SinkClear::NothingToClear))
    } else if presence == SinkPresence::Gone {
        Some((SinkClearSkip::SinkGone, SinkClear::SkippedSinkGone))
    } else {
        None
    };

    let ordered = order(config);
    let parts: &[Part] = if config.clear_sink {
        &ordered
    } else {
        // Clearing off: only the output stops. The sink part records why it was skipped.
        &[Part::Sink, Part::Output]
    };

    let mut sink = SinkClear::NotRequested;
    let mut scrambler = None;
    let mut output = None;
    for &part in parts {
        match part {
            Part::Sink => match skip.take() {
                Some((reason, result)) => {
                    emit(io, TmdsEventKind::SinkClearSkipped { reason });
                    sink = result;
                }
                None => {
                    let cleared = SinkTmdsConfig::new(false, false);
                    match io.write_tmds_config(cleared).await {
                        Ok(()) => {
                            emit(io, TmdsEventKind::TmdsConfigWritten { config: cleared });
                            sink = SinkClear::Cleared;
                        }
                        Err(error) => {
                            emit(
                                io,
                                TmdsEventKind::ScdcStepFailed {
                                    step: TmdsStep::WriteTmdsConfig,
                                    kind: error.kind(),
                                    ends: false,
                                },
                            );
                            sink = SinkClear::Failed(error);
                        }
                    }
                }
            },
            Part::Scrambler => match io.set_scrambling(false).await {
                Ok(()) => emit(io, TmdsEventKind::ScramblerSet { enabled: false }),
                Err(error) => {
                    emit(
                        io,
                        TmdsEventKind::PhyStepFailed {
                            step: TmdsStep::SetScrambling,
                            ends: false,
                        },
                    );
                    scrambler = Some(error);
                }
            },
            Part::Output => match io.stop_output().await {
                Ok(()) => emit(io, TmdsEventKind::OutputStopped),
                Err(error) => {
                    emit(
                        io,
                        TmdsEventKind::PhyStepFailed {
                            step: TmdsStep::StopOutput,
                            ends: false,
                        },
                    );
                    output = Some(error);
                }
            },
        }
    }

    emit(
        io,
        TmdsEventKind::TearDownEnded {
            sink: sink.kinds(),
            scrambler_failed: scrambler.is_some(),
            output_failed: output.is_some(),
        },
    );
    if scrambler.is_none() && output.is_none() {
        Ok(TornDown { sink })
    } else {
        Err(TearDownError {
            sink,
            scrambler,
            output,
        })
    }
}

#[cfg(test)]
mod tests {
    use hdmi_hal::ddc::DdcErrorKind;

    use super::*;
    use crate::MAX_TEAR_DOWN_EVENTS;
    use crate::procedure::test_io::{Op, TestIo, block_on};
    use crate::types::{SinkTmdsFacts, SourceTmdsFacts, TmdsPolicy};

    /// 4K60 RGB 8 bpc: scrambled at 1/40.
    fn scrambled() -> TmdsLinkParams {
        TmdsLinkParams::choose(
            594_000,
            SinkTmdsFacts::new(true, true, false),
            SourceTmdsFacts::new(false),
            TmdsPolicy::DEFAULT,
        )
        .unwrap()
    }

    /// 1080p60 on a sink with SCDC that does not declare low-rate scrambling: plain.
    fn plain() -> TmdsLinkParams {
        TmdsLinkParams::choose(
            148_500,
            SinkTmdsFacts::new(true, true, false),
            SourceTmdsFacts::new(false),
            TmdsPolicy::DEFAULT,
        )
        .unwrap()
    }

    const CLEARING: TearDownConfig = TearDownConfig::DEFAULT.with_clear_sink(true);

    fn run(
        io: &mut TestIo,
        link: &TmdsLinkParams,
        config: &TearDownConfig,
        presence: SinkPresence,
    ) -> (
        Result<TornDown<DdcErrorKind>, TearDownError<DdcErrorKind, ()>>,
        heapless_events::Events,
    ) {
        let mut events = heapless_events::Events::default();
        let result = block_on(tear_down(io, link, config, presence, &mut |e| {
            events.push(e)
        }));
        (result, events)
    }

    /// A fixed buffer of events, as a caller without `alloc` keeps them.
    mod heapless_events {
        use crate::event::{TmdsEvent, TmdsEventKind};

        #[derive(Default)]
        pub struct Events {
            pub list: [Option<TmdsEvent>; 8],
            pub len: usize,
        }

        impl Events {
            pub fn push(&mut self, event: TmdsEvent) {
                self.list[self.len] = Some(event);
                self.len += 1;
            }

            pub fn kinds(&self) -> impl Iterator<Item = TmdsEventKind> + '_ {
                self.list[..self.len].iter().map(|e| e.unwrap().kind)
            }
        }
    }

    #[test]
    fn by_default_only_the_output_stops() {
        let mut io = TestIo::new();
        let (result, events) = run(
            &mut io,
            &scrambled(),
            &TearDownConfig::DEFAULT,
            SinkPresence::Present,
        );
        assert_eq!(
            result,
            Ok(TornDown {
                sink: SinkClear::NotRequested
            })
        );
        assert_eq!(io.ops(), [Op::StopOutput]);
        let kinds: [TmdsEventKind; 4] = core::array::from_fn(|i| events.kinds().nth(i).unwrap());
        assert!(matches!(kinds[0], TmdsEventKind::TearDownStarted { .. }));
        assert_eq!(
            kinds[1],
            TmdsEventKind::SinkClearSkipped {
                reason: SinkClearSkip::NotRequested
            }
        );
        assert_eq!(kinds[2], TmdsEventKind::OutputStopped);
        assert_eq!(
            kinds[3],
            TmdsEventKind::TearDownEnded {
                sink: SinkClear::NotRequested,
                scrambler_failed: false,
                output_failed: false,
            }
        );
        assert_eq!(events.len, 4);
    }

    #[test]
    fn clearing_follows_the_order_the_settings_give() {
        let cleared = Op::WriteTmdsConfig(SinkTmdsConfig::new(false, false));
        let cases = [
            (
                SinkClearTiming::BeforeOutputStops,
                ScramblerOff::AfterSink,
                [cleared, Op::SetScrambling(false), Op::StopOutput],
            ),
            (
                SinkClearTiming::BeforeOutputStops,
                ScramblerOff::BeforeSink,
                [Op::SetScrambling(false), cleared, Op::StopOutput],
            ),
            (
                SinkClearTiming::AfterOutputStops,
                ScramblerOff::AfterSink,
                [Op::StopOutput, cleared, Op::SetScrambling(false)],
            ),
            (
                SinkClearTiming::AfterOutputStops,
                ScramblerOff::BeforeSink,
                [Op::SetScrambling(false), Op::StopOutput, cleared],
            ),
        ];
        for (timing, scrambler_off, expected) in cases {
            let config = CLEARING
                .with_sink_clear_timing(timing)
                .with_scrambler_off(scrambler_off);
            let mut io = TestIo::new();
            let (result, events) = run(&mut io, &scrambled(), &config, SinkPresence::Present);
            assert_eq!(
                result,
                Ok(TornDown {
                    sink: SinkClear::Cleared
                })
            );
            assert_eq!(io.ops(), expected, "{timing:?} {scrambler_off:?}");
            assert_eq!(events.len, MAX_TEAR_DOWN_EVENTS);
        }
    }

    #[test]
    fn nothing_is_written_to_a_plain_link_a_sink_without_scdc_or_a_gone_sink() {
        let no_scdc = TmdsLinkParams::choose(
            148_500,
            SinkTmdsFacts::NONE,
            SourceTmdsFacts::new(false),
            TmdsPolicy::DEFAULT,
        )
        .unwrap();
        for (link, presence, expected, reason) in [
            (
                plain(),
                SinkPresence::Present,
                SinkClear::NothingToClear,
                SinkClearSkip::NothingToClear,
            ),
            (
                no_scdc,
                SinkPresence::Present,
                SinkClear::NothingToClear,
                SinkClearSkip::NothingToClear,
            ),
            (
                scrambled(),
                SinkPresence::Gone,
                SinkClear::SkippedSinkGone,
                SinkClearSkip::SinkGone,
            ),
        ] {
            let mut io = TestIo::new();
            let (result, events) = run(&mut io, &link, &CLEARING, presence);
            assert_eq!(result, Ok(TornDown { sink: expected }));
            assert_eq!(io.ops(), [Op::SetScrambling(false), Op::StopOutput]);
            assert!(
                events
                    .kinds()
                    .any(|k| k == TmdsEventKind::SinkClearSkipped { reason })
            );
        }
    }

    #[test]
    fn a_failed_sink_write_is_a_warning() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        let (result, events) = run(&mut io, &scrambled(), &CLEARING, SinkPresence::Present);
        assert_eq!(
            result,
            Ok(TornDown {
                sink: SinkClear::Failed(DdcErrorKind::Busy)
            })
        );
        // The write was attempted and failed; the rest went on.
        assert_eq!(
            io.ops(),
            [
                Op::WriteTmdsConfig(SinkTmdsConfig::new(false, false)),
                Op::SetScrambling(false),
                Op::StopOutput
            ]
        );
        assert!(events.kinds().any(|k| k
            == TmdsEventKind::ScdcStepFailed {
                step: TmdsStep::WriteTmdsConfig,
                kind: DdcErrorKind::Busy,
                ends: false,
            }));
    }

    #[test]
    fn a_failed_phy_step_does_not_keep_the_output_running() {
        let mut io = TestIo::new();
        io.fail_set_scrambling = true;
        let (result, events) = run(&mut io, &scrambled(), &CLEARING, SinkPresence::Present);
        assert_eq!(
            result,
            Err(TearDownError {
                sink: SinkClear::Cleared,
                scrambler: Some(()),
                output: None,
            })
        );
        assert_eq!(io.ops().last(), Some(&Op::StopOutput));
        assert!(events.kinds().any(|k| k
            == TmdsEventKind::TearDownEnded {
                sink: SinkClear::Cleared,
                scrambler_failed: true,
                output_failed: false,
            }));

        let mut io = TestIo::new();
        io.fail_stop_output = true;
        let (result, events) = run(&mut io, &scrambled(), &CLEARING, SinkPresence::Present);
        assert_eq!(
            result,
            Err(TearDownError {
                sink: SinkClear::Cleared,
                scrambler: None,
                output: Some(()),
            })
        );
        assert!(events.kinds().any(|k| k
            == TmdsEventKind::PhyStepFailed {
                step: TmdsStep::StopOutput,
                ends: false,
            }));
    }

    #[test]
    fn no_combination_records_more_than_the_bound() {
        for clear_sink in [false, true] {
            for timing in [
                SinkClearTiming::BeforeOutputStops,
                SinkClearTiming::AfterOutputStops,
            ] {
                for off in [ScramblerOff::AfterSink, ScramblerOff::BeforeSink] {
                    for presence in [SinkPresence::Present, SinkPresence::Gone] {
                        for link in [scrambled(), plain()] {
                            for fail in [false, true] {
                                let config = TearDownConfig::DEFAULT
                                    .with_clear_sink(clear_sink)
                                    .with_sink_clear_timing(timing)
                                    .with_scrambler_off(off);
                                let mut io = TestIo::new();
                                if fail {
                                    io.fail_scdc = Some(DdcErrorKind::Other);
                                    io.fail_set_scrambling = true;
                                    io.fail_stop_output = true;
                                }
                                let (_, events) = run(&mut io, &link, &config, presence);
                                assert!(events.len <= MAX_TEAR_DOWN_EVENTS);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn events_are_stamped_from_the_io_clock() {
        let mut io = TestIo::new();
        let (_, events) = run(&mut io, &scrambled(), &CLEARING, SinkPresence::Present);
        let stamps: [u64; 5] = core::array::from_fn(|i| events.list[i].unwrap().at.as_micros());
        assert!(stamps.windows(2).all(|w| w[0] < w[1]), "{stamps:?}");
    }
}
