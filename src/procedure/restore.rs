//! The quick restore: after the reconnect check found the sink's `TMDS_Config` lost, write it
//! again with the output running, set the source's scrambler again, and check.

use hdmi_hal::ddc::DdcError;

use crate::config::{BringUpConfig, OnTmdsConfigFailure, TmdsConfigWrite};
use crate::error::UpdateError;
use crate::event::{TmdsConfigSkip, TmdsEvent, TmdsEventKind, TmdsStep};
use crate::outcome::{RestoreError, Restored, ScramblingCheck};
use crate::params::TmdsLinkParams;

use super::TmdsIo;
use super::bring_up::transient;
use super::check::scrambling_check;

/// Restores the sink's `TMDS_Config` on the running link `link` without a new bring-up: the
/// option to a tear-down and a bring-up after
/// [`ReconnectCheck::Lost`](crate::ReconnectCheck::Lost).
///
/// In order: the sink's `TMDS_Config` in one write ([`TmdsLinkParams::sink_config`]; none
/// when no write is due), with the output left running and no wait, as the ratio does not
/// change; the source's scrambler set again to the link's, in case the source lost its
/// state with the sink; and, on a scrambled link, bring-up's scrambling check. It does not
/// touch the PHY's rate or clock ratio and does not stop the output.
///
/// From `config` it reads `tmds_config_write`, `on_tmds_config_failure`, `check_window` and
/// `check_interval`, and ignores the rest. A failed write under
/// [`OnTmdsConfigFailure::CarryOn`] is a warning, [`Restored::write_failed`].
///
/// The cancel flag is read by the check alone, before each of its reads: the write and the
/// scrambler are one call each, and a restore stopped between them would leave the sink
/// and the source disagreeing. A cancelled check is
/// [`CheckResult::Cancelled`](crate::CheckResult::Cancelled), with the link left running.
///
/// Records at most [`MAX_QUICK_RESTORE_EVENTS`](crate::MAX_QUICK_RESTORE_EVENTS) events.
///
/// # Errors
///
/// [`RestoreError::Scdc`] when the write failed with a non-transient error (`Detached`,
/// `Unsupported`), or at all under [`OnTmdsConfigFailure::Stop`]; nothing follows it.
/// [`RestoreError::Phy`] when setting the scrambler failed. No tear-down follows either:
/// the link was running, and what to do next is the caller's.
pub async fn quick_restore<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    link: &TmdsLinkParams,
    config: &BringUpConfig,
    record: &mut F,
) -> Result<Restored<Io::ScdcError>, RestoreError<Io::ScdcError, Io::PhyError>> {
    let mut emit = |io: &Io, kind| record(TmdsEvent::new(io.now(), kind));
    emit(io, TmdsEventKind::QuickRestoreStarted { link: *link });
    let end = |failed_at| TmdsEventKind::QuickRestoreEnded { failed_at };

    let mut sink_written_at = None;
    let mut write_failed = None;
    if let Some(value) = link.sink_config() {
        let result = match config.tmds_config_write {
            TmdsConfigWrite::WholeByte => io
                .write_tmds_config(value)
                .await
                .map_err(UpdateError::Write),
            _ => io.update_tmds_config(value).await,
        };
        sink_written_at = Some(io.now());
        match result {
            Ok(()) => emit(io, TmdsEventKind::TmdsConfigWritten { config: value }),
            Err(error) => {
                // The step names the half: a read-modify-write whose read failed wrote
                // nothing.
                let step = match error {
                    UpdateError::Read(_) => TmdsStep::ReadTmdsConfig,
                    UpdateError::Write(_) => TmdsStep::WriteTmdsConfig,
                };
                let kind = error.kind();
                let ends =
                    !transient(kind) || config.on_tmds_config_failure == OnTmdsConfigFailure::Stop;
                emit(io, TmdsEventKind::ScdcStepFailed { step, kind, ends });
                if ends {
                    emit(io, end(Some(step)));
                    return Err(RestoreError::Scdc(error));
                }
                write_failed = Some(error);
            }
        }
    } else {
        let reason = if link.sink().scdc_present {
            TmdsConfigSkip::Gate
        } else {
            TmdsConfigSkip::NoScdc
        };
        emit(io, TmdsEventKind::TmdsConfigSkipped { reason });
    }

    let scrambled = link.scrambling();
    if let Err(error) = io.set_scrambling(scrambled).await {
        let step = TmdsStep::SetScrambling;
        emit(io, TmdsEventKind::PhyStepFailed { step, ends: true });
        emit(io, end(Some(step)));
        return Err(RestoreError::Phy {
            error,
            sink_written_at,
            write_failed,
        });
    }
    emit(io, TmdsEventKind::ScramblerSet { enabled: scrambled });

    let check = if scrambled {
        let from = io.now();
        scrambling_check(
            io,
            from,
            config.check_window,
            config.check_interval,
            &mut |e| record(e),
        )
        .await
    } else {
        ScramblingCheck::NOT_APPLICABLE
    };
    record(TmdsEvent::new(io.now(), end(None)));
    Ok(Restored {
        sink_written_at,
        check,
        write_failed,
    })
}

#[cfg(test)]
mod tests {
    use hdmi_hal::ddc::DdcErrorKind;

    use super::*;
    use crate::MAX_QUICK_RESTORE_EVENTS;
    use crate::outcome::CheckResult;
    use crate::procedure::test_io::{Op, TestIo, block_on};
    use crate::types::{SinkTmdsConfig, SinkTmdsFacts, SourceTmdsFacts, TmdsPolicy};

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

    type Outcome = Result<Restored<DdcErrorKind>, RestoreError<DdcErrorKind, ()>>;

    fn restore(
        io: &mut TestIo,
        link: &TmdsLinkParams,
        config: &BringUpConfig,
    ) -> (Outcome, usize, Option<TmdsEventKind>) {
        let mut count = 0;
        let mut last = None;
        let result = block_on(quick_restore(io, link, config, &mut |e| {
            count += 1;
            last = Some(e.kind);
        }));
        assert!(count <= MAX_QUICK_RESTORE_EVENTS, "{count} events");
        (result, count, last)
    }

    #[test]
    fn a_scrambled_link_is_written_set_and_checked_with_the_output_running() {
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        let (result, count, last) = restore(&mut io, &link(594_000, SINK), &BringUpConfig::DEFAULT);
        assert_eq!(
            io.ops(),
            [
                Op::UpdateTmdsConfig(SinkTmdsConfig::new(true, true)),
                Op::SetScrambling(true),
                Op::ReadScramblingStatus,
            ]
        );
        let restored = result.unwrap();
        assert!(restored.sink_written_at.is_some());
        assert_eq!(restored.check.result, CheckResult::Set);
        assert_eq!(restored.write_failed, None);
        assert_eq!(count, MAX_QUICK_RESTORE_EVENTS);
        assert_eq!(
            last,
            Some(TmdsEventKind::QuickRestoreEnded { failed_at: None })
        );
    }

    #[test]
    fn a_plain_link_is_written_and_not_checked() {
        let mut io = TestIo::new();
        let config = BringUpConfig::DEFAULT.with_tmds_config_write(TmdsConfigWrite::WholeByte);
        let (result, ..) = restore(&mut io, &link(148_500, SINK), &config);
        assert_eq!(
            io.ops(),
            [
                Op::WriteTmdsConfig(SinkTmdsConfig::new(false, false)),
                Op::SetScrambling(false),
            ]
        );
        assert_eq!(result.unwrap().check.result, CheckResult::NotApplicable);
    }

    #[test]
    fn no_write_is_made_where_none_is_due() {
        for (sink, reason) in [
            (SinkTmdsFacts::NONE, TmdsConfigSkip::NoScdc),
            (SinkTmdsFacts::new(true, false, false), TmdsConfigSkip::Gate),
        ] {
            let policy = TmdsPolicy::DEFAULT
                .with_tmds_config_gate(crate::types::TmdsConfigGate::ScramblingSink);
            let link =
                TmdsLinkParams::choose(148_500, sink, SourceTmdsFacts::new(false), policy).unwrap();
            let mut io = TestIo::new();
            let mut skipped = false;
            let result = block_on(quick_restore(
                &mut io,
                &link,
                &BringUpConfig::DEFAULT,
                &mut |e| skipped |= e.kind == TmdsEventKind::TmdsConfigSkipped { reason },
            ));
            assert!(skipped);
            assert_eq!(io.ops(), [Op::SetScrambling(false)]);
            assert_eq!(result.unwrap().sink_written_at, None);
        }
    }

    #[test]
    fn a_failed_write_carries_on_or_stops() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.scdc_failures = 1;
        io.update_fails_at_write = true;
        io.scrambling_status = 0x01;
        let (result, ..) = restore(&mut io, &link(594_000, SINK), &BringUpConfig::DEFAULT);
        let restored = result.unwrap();
        assert_eq!(
            restored.write_failed,
            Some(UpdateError::Write(DdcErrorKind::Busy))
        );
        assert!(restored.sink_written_at.is_some());
        assert_eq!(restored.check.result, CheckResult::Set);

        let stop = BringUpConfig::DEFAULT.with_on_tmds_config_failure(OnTmdsConfigFailure::Stop);
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        let (result, _, last) = restore(&mut io, &link(594_000, SINK), &stop);
        assert_eq!(
            result,
            Err(RestoreError::Scdc(UpdateError::Read(DdcErrorKind::Busy)))
        );
        assert_eq!(io.ops().len(), 1);
        assert_eq!(
            last,
            Some(TmdsEventKind::QuickRestoreEnded {
                failed_at: Some(TmdsStep::ReadTmdsConfig)
            })
        );

        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Detached);
        let (result, ..) = restore(&mut io, &link(594_000, SINK), &BringUpConfig::DEFAULT);
        assert!(matches!(result, Err(RestoreError::Scdc(_))));
    }

    #[test]
    fn a_failed_scrambler_call_is_an_error_with_the_write() {
        let mut io = TestIo::new();
        io.fail_set_scrambling = true;
        io.fail_scdc = Some(DdcErrorKind::Busy);
        let (result, count, _) = restore(&mut io, &link(594_000, SINK), &BringUpConfig::DEFAULT);
        assert!(matches!(
            result,
            Err(RestoreError::Phy {
                error: (),
                sink_written_at: Some(_),
                write_failed: Some(UpdateError::Read(DdcErrorKind::Busy)),
            })
        ));
        assert_eq!(count, 4);
    }

    #[test]
    fn only_the_check_reads_the_flag() {
        let mut io = TestIo::new();
        io.cancelled = true;
        let (result, ..) = restore(&mut io, &link(594_000, SINK), &BringUpConfig::DEFAULT);
        let restored = result.unwrap();
        assert_eq!(restored.check.result, CheckResult::Cancelled);
        assert_eq!(restored.check.reads, 0);
        assert_eq!(io.ops().len(), 2);
        // A sink that never reports scrambling: the check runs its window.
        let mut io = TestIo::new();
        let (result, ..) = restore(&mut io, &link(594_000, SINK), &BringUpConfig::DEFAULT);
        let check = result.unwrap().check;
        assert_eq!(check.result, CheckResult::NotSet);
        assert!(check.set_after.is_none() && check.reads > 1);
    }
}
