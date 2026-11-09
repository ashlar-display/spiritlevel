//! The reconnect check: after a hotplug while the link is up, whether the sink still holds
//! the `TMDS_Config` the link needs.

use hdmi_hal::ddc::DdcError;

use crate::event::{TmdsEvent, TmdsEventKind, TmdsStep};
use crate::outcome::ReconnectCheck;
use crate::params::TmdsLinkParams;
use crate::types::SinkTmdsFacts;

use super::TmdsIo;

/// Checks whether the sink still holds the `TMDS_Config` `link` needs, after a hotplug
/// while the link is up, or at start-up on a link found running.
///
/// `current_sink` is the sink's facts now, from the EDID read after the hotplug, with any
/// quirk the caller applied to the first reading applied again. When the link no longer
/// fits them ([`TmdsLinkParams::fits`]: choosing again would refuse the rate or give another
/// ratio, scrambling or `TMDS_Config`), the result is [`ReconnectCheck::SinkChanged`], with
/// nothing read; a fact the link does not depend on, such as FRL, may change. Facts that fit
/// do not prove the same sink; telling sinks apart by their EDID is the caller's.
///
/// Otherwise one read, compared with [`TmdsLinkParams::sink_config`] on both bits, a plain
/// link included (its expected value is both bits 0). A link with no write due (a sink
/// without SCDC, or a plain link the policy's gate leaves unwritten) is
/// [`ReconnectCheck::NotApplicable`]. The check writes nothing and never changes the link;
/// a failed read is reported, not acted on. Not cancelled: it is one read.
///
/// Records at most [`MAX_RECONNECT_CHECK_EVENTS`](crate::MAX_RECONNECT_CHECK_EVENTS) events.
pub async fn check_reconnect<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    link: &TmdsLinkParams,
    current_sink: SinkTmdsFacts,
    record: &mut F,
) -> ReconnectCheck<Io::ScdcError> {
    let mut emit = |io: &Io, kind| record(TmdsEvent::new(io.now(), kind));
    emit(
        io,
        TmdsEventKind::ReconnectCheckStarted {
            link: *link,
            current_sink,
        },
    );

    let result = if !link.fits(current_sink) {
        ReconnectCheck::SinkChanged
    } else if let Some(expected) = link.sink_config() {
        match io.read_tmds_config().await {
            Ok(read) => {
                emit(io, TmdsEventKind::TmdsConfigRead { reading: read });
                if read.config == expected {
                    ReconnectCheck::Matches
                } else {
                    ReconnectCheck::Lost { read }
                }
            }
            Err(error) => {
                emit(
                    io,
                    TmdsEventKind::ScdcStepFailed {
                        step: TmdsStep::ReadTmdsConfig,
                        kind: error.kind(),
                        ends: true,
                    },
                );
                ReconnectCheck::Unreadable(error)
            }
        }
    } else {
        ReconnectCheck::NotApplicable
    };

    emit(
        io,
        TmdsEventKind::ReconnectChecked {
            result: result.kinds(),
        },
    );
    result
}

#[cfg(test)]
mod tests {
    use hdmi_hal::ddc::DdcErrorKind;

    use super::*;
    use crate::MAX_RECONNECT_CHECK_EVENTS;
    use crate::procedure::test_io::{Op, TestIo, block_on};
    use crate::types::{SinkTmdsConfig, SourceTmdsFacts, TmdsPolicy};

    const SINK: SinkTmdsFacts = SinkTmdsFacts::new(true, true, false);

    fn link(rate_khz: u32, sink: SinkTmdsFacts) -> TmdsLinkParams {
        TmdsLinkParams::choose(
            rate_khz,
            sink,
            SourceTmdsFacts::new(false),
            TmdsPolicy::DEFAULT,
        )
        .unwrap()
    }

    fn check(
        io: &mut TestIo,
        link: &TmdsLinkParams,
        current: SinkTmdsFacts,
    ) -> (ReconnectCheck<DdcErrorKind>, usize, Option<TmdsEventKind>) {
        let mut count = 0;
        let mut last = None;
        let result = block_on(check_reconnect(io, link, current, &mut |e| {
            count += 1;
            last = Some(e.kind);
        }));
        (result, count, last)
    }

    #[test]
    fn a_sink_that_kept_its_config_matches() {
        let mut io = TestIo::new();
        io.tmds_config = 0x03;
        let (result, count, last) = check(&mut io, &link(594_000, SINK), SINK);
        assert_eq!(result, ReconnectCheck::Matches);
        assert_eq!(io.ops(), [Op::ReadTmdsConfig]);
        assert_eq!(count, MAX_RECONNECT_CHECK_EVENTS);
        assert_eq!(
            last,
            Some(TmdsEventKind::ReconnectChecked {
                result: ReconnectCheck::Matches
            })
        );
    }

    #[test]
    fn a_reset_sink_lost_its_config_and_the_bits_read_are_kept() {
        let mut io = TestIo::new();
        io.tmds_config = 0x80; // both bits 0, a reserved bit set
        let (result, ..) = check(&mut io, &link(594_000, SINK), SINK);
        assert!(matches!(
            result,
            ReconnectCheck::Lost { read }
                if read.config == SinkTmdsConfig::new(false, false) && read.raw == 0x80
        ));
    }

    #[test]
    fn both_bits_are_compared() {
        let mut io = TestIo::new();
        io.tmds_config = 0x01; // scrambling kept, the ratio lost
        let (result, ..) = check(&mut io, &link(594_000, SINK), SINK);
        assert!(matches!(result, ReconnectCheck::Lost { .. }));
    }

    #[test]
    fn a_plain_link_expects_both_bits_clear() {
        let plain = link(148_500, SINK);
        let mut io = TestIo::new();
        assert_eq!(check(&mut io, &plain, SINK).0, ReconnectCheck::Matches);
        io.tmds_config = 0x03;
        assert!(matches!(
            check(&mut io, &plain, SINK).0,
            ReconnectCheck::Lost { .. }
        ));
    }

    #[test]
    fn a_failed_read_is_reported_not_acted_on() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Detached);
        let (result, count, _) = check(&mut io, &link(594_000, SINK), SINK);
        assert_eq!(result, ReconnectCheck::Unreadable(DdcErrorKind::Detached));
        assert_eq!(io.ops(), [Op::ReadTmdsConfig]);
        assert_eq!(count, MAX_RECONNECT_CHECK_EVENTS);
    }

    #[test]
    fn changed_facts_or_no_scdc_read_nothing() {
        let mut io = TestIo::new();
        let other = SinkTmdsFacts::new(true, false, false);
        let (result, count, _) = check(&mut io, &link(594_000, SINK), other);
        assert_eq!(result, ReconnectCheck::SinkChanged);
        assert_eq!(count, 2);

        // A fact the link does not depend on changes nothing: FRL toggled in the EDID, or
        // LTE_340Mcsc_scramble on a 1/40 link.
        io.tmds_config = 0x03;
        let frl_off = SINK.with_frl_capable(false);
        assert_eq!(
            check(&mut io, &link(594_000, SINK), frl_off).0,
            ReconnectCheck::Matches
        );
        let lte = SinkTmdsFacts::new(true, true, true);
        assert_eq!(
            check(&mut io, &link(594_000, SINK), lte).0,
            ReconnectCheck::Matches
        );
        // On a low-rate link it is what the link depends on.
        let low = TmdsLinkParams::choose(
            297_000,
            lte,
            SourceTmdsFacts::new(true),
            TmdsPolicy::DEFAULT,
        )
        .unwrap();
        assert!(low.scrambling());
        assert_eq!(check(&mut io, &low, SINK).0, ReconnectCheck::SinkChanged);
        io = TestIo::new();

        let no_scdc = link(148_500, SinkTmdsFacts::NONE);
        let (result, count, _) = check(&mut io, &no_scdc, SinkTmdsFacts::NONE);
        assert_eq!(result, ReconnectCheck::NotApplicable);
        assert_eq!(count, 2);
        assert_eq!(io.ops().len(), 0);
    }
}
