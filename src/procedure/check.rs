//! The scrambling check: poll `Scrambling_Status` until the sink reports scrambled TMDS or
//! the window ends.

use core::time::Duration;

use hdmi_hal::ddc::{DdcError, DdcErrorKind};
use hdmi_hal::time::Timestamp;

use crate::event::{TmdsEvent, TmdsEventKind};
use crate::outcome::{CheckResult, FailedRead, ScramblingCheck};

use super::TmdsIo;

/// The most reads one check makes: `ceil(window / interval) + 1`, so a clock that does not
/// advance ends the check instead of hanging it. With an interval of 0 (back-to-back reads),
/// one read per microsecond of the window. It bounds the number of reads, not the time: on
/// a clock that does not advance, the check takes as long as that many reads take, which
/// with a tiny interval is long ([`BringUpConfig::validate`](crate::BringUpConfig::validate)
/// reports an interval of 0).
fn read_cap(window: Duration, interval: Duration) -> u32 {
    let per = if interval.is_zero() {
        Duration::from_micros(1)
    } else {
        interval
    };
    let reads = window.as_nanos().div_ceil(per.as_nanos()) + 1;
    u32::try_from(reads).unwrap_or(u32::MAX)
}

/// Polls `Scrambling_Status` from `from` until the sink reports scrambling, the window ends
/// or the read count is reached. At least one read; no later read starts after the window:
/// the check ends when another interval would reach the window's end, and again if the
/// delay itself ran past it. A slow read that starts inside the window may still end after
/// it. A failed
/// read does not end the check unless its error is not transient (`Detached`,
/// `Unsupported`). The cancel flag is read before every read; once set, the check ends
/// [`CheckResult::Cancelled`]. Records one event, at its end.
pub(crate) async fn scrambling_check<Io: TmdsIo, F: FnMut(TmdsEvent)>(
    io: &mut Io,
    from: Timestamp,
    window: Duration,
    interval: Duration,
    record: &mut F,
) -> ScramblingCheck<Io::ScdcError> {
    let cap = read_cap(window, interval);
    let mut check = ScramblingCheck {
        result: CheckResult::NotSet,
        reads: 0,
        failed_reads: 0,
        first_error: None,
        set_after: None,
        read_cap_reached: false,
    };
    loop {
        if check.reads > 0 {
            // The window first: the read count marks a clock that did not advance.
            let elapsed = io.now().saturating_duration_since(from);
            if elapsed.saturating_add(interval) >= window {
                break;
            }
            if check.reads >= cap {
                check.read_cap_reached = true;
                break;
            }
            io.delay(interval).await;
            // A delay is "at least" its duration: one that ran late ends the check too.
            if io.now().saturating_duration_since(from) >= window {
                break;
            }
        }
        if io.cancelled() {
            check.result = CheckResult::Cancelled;
            break;
        }
        let read = io.read_scrambling_status().await;
        check.reads += 1;
        let after = io.now().saturating_duration_since(from);
        match read {
            Ok(status) if status.set => {
                check.result = CheckResult::Set;
                check.set_after = Some(after);
                break;
            }
            Ok(_) => {}
            Err(error) => {
                check.failed_reads += 1;
                let kind = error.kind();
                if check.first_error.is_none() {
                    check.first_error = Some(FailedRead { error, after });
                }
                if matches!(kind, DdcErrorKind::Detached | DdcErrorKind::Unsupported) {
                    check.result = CheckResult::Unreadable;
                    break;
                }
            }
        }
    }
    if check.result == CheckResult::NotSet && check.failed_reads == check.reads {
        check.result = CheckResult::Unreadable;
    }
    record(TmdsEvent::new(
        io.now(),
        TmdsEventKind::ScramblingChecked {
            check: check.kinds(),
        },
    ));
    check
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procedure::test_io::{Op, TestIo, block_on};

    const WINDOW: Duration = Duration::from_millis(200);
    const INTERVAL: Duration = Duration::from_millis(1);

    fn run(io: &mut TestIo) -> (ScramblingCheck<DdcErrorKind>, Option<TmdsEventKind>) {
        let from = io.now();
        let mut last = None;
        let check = block_on(scrambling_check(io, from, WINDOW, INTERVAL, &mut |e| {
            last = Some(e.kind);
        }));
        (check, last)
    }

    #[test]
    fn a_sink_that_reports_at_once_is_set_after_one_read() {
        let mut io = TestIo::new();
        io.scrambling_status = 0x01;
        let (check, event) = run(&mut io);
        assert_eq!(check.result, CheckResult::Set);
        assert_eq!((check.reads, check.failed_reads), (1, 0));
        assert!(check.set_after.is_some());
        assert_eq!(event, Some(TmdsEventKind::ScramblingChecked { check }));
    }

    #[test]
    fn a_sink_that_reports_late_is_set_when_it_does() {
        let mut io = TestIo::new();
        io.status_set_from_read = Some(5);
        let (check, _) = run(&mut io);
        assert_eq!(check.result, CheckResult::Set);
        assert_eq!(check.reads, 5);
        assert!(check.set_after.unwrap() >= Duration::from_millis(4));
    }

    #[test]
    fn a_sink_that_never_reports_is_not_set_after_the_window() {
        let mut io = TestIo::new();
        let (check, _) = run(&mut io);
        assert_eq!(check.result, CheckResult::NotSet);
        assert!(check.reads > 150 && check.reads <= 201, "{}", check.reads);
        assert!(!check.read_cap_reached);
    }

    #[test]
    fn failed_reads_do_not_end_the_check() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::NoAcknowledge(
            hdmi_hal::ddc::NoAcknowledgeSource::Address,
        ));
        io.scdc_failures = 3;
        io.status_set_from_read = Some(1);
        let (check, _) = run(&mut io);
        assert_eq!(check.result, CheckResult::Set);
        assert_eq!((check.reads, check.failed_reads), (4, 3));
        assert!(check.first_error.is_some_and(|f| f.after > Duration::ZERO));
    }

    #[test]
    fn every_read_failing_is_unreadable_and_a_gone_bus_ends_it_at_once() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        let (check, _) = run(&mut io);
        assert_eq!(check.result, CheckResult::Unreadable);
        assert_eq!(check.reads, check.failed_reads);
        assert!(check.reads > 1);

        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Detached);
        let (check, event) = run(&mut io);
        assert_eq!(check.result, CheckResult::Unreadable);
        assert_eq!(check.reads, 1);
        assert!(matches!(
            event,
            Some(TmdsEventKind::ScramblingChecked { check }) if check.first_error.map(|f| f.error) == Some(DdcErrorKind::Detached)
        ));
    }

    #[test]
    fn unsupported_ends_it_at_once_and_some_failed_reads_are_not_unreadable() {
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Unsupported);
        let (check, _) = run(&mut io);
        assert_eq!((check.result, check.reads), (CheckResult::Unreadable, 1));

        // Two failed reads, then a sink that answers but never reports scrambling.
        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.scdc_failures = 2;
        let (check, _) = run(&mut io);
        assert_eq!(check.result, CheckResult::NotSet);
        assert_eq!(check.failed_reads, 2);
        assert!(check.reads > 2);
    }

    #[test]
    fn a_frozen_clock_ends_the_check_on_its_read_count() {
        let mut io = TestIo::new();
        io.frozen_clock = true;
        let (check, _) = run(&mut io);
        assert_eq!(check.result, CheckResult::NotSet);
        assert!(check.read_cap_reached);
        assert_eq!(check.reads, 201);
    }

    #[test]
    fn the_flag_is_read_before_every_read() {
        let mut io = TestIo::new();
        io.cancelled = true;
        let (check, event) = run(&mut io);
        assert_eq!((check.result, check.reads), (CheckResult::Cancelled, 0));
        assert_eq!(event, Some(TmdsEventKind::ScramblingChecked { check }));

        let mut io = TestIo::new();
        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.cancel_after_ops = Some(5); // three reads and the delays between them
        let (check, _) = run(&mut io);
        assert_eq!(check.result, CheckResult::Cancelled);
        assert_eq!((check.reads, check.failed_reads), (3, 3));
    }

    #[test]
    fn no_read_starts_after_the_window() {
        let window = Duration::from_millis(10);
        let interval = Duration::from_millis(3);
        let mut io = TestIo::new();
        let from = io.now();
        let check = block_on(scrambling_check(
            &mut io,
            from,
            window,
            interval,
            &mut |_| {},
        ));
        // Reads at 0, 3, 6 and 9 ms; another interval would reach 12 ms.
        assert_eq!((check.result, check.reads), (CheckResult::NotSet, 4));
        assert!(io.now().saturating_duration_since(from) < Duration::from_millis(11));

        // A sink that would report at the fifth read is not seen.
        let mut io = TestIo::new();
        io.status_set_from_read = Some(5);
        let from = io.now();
        let check = block_on(scrambling_check(
            &mut io,
            from,
            window,
            interval,
            &mut |_| {},
        ));
        assert_eq!(check.result, CheckResult::NotSet);

        // An interval as long as the window, or longer, reads once.
        for interval in [window, window * 2] {
            let mut io = TestIo::new();
            let from = io.now();
            let check = block_on(scrambling_check(
                &mut io,
                from,
                window,
                interval,
                &mut |_| {},
            ));
            assert_eq!(check.reads, 1);
            assert!(!io.ops().iter().any(|op| matches!(op, Op::Delay(_))));
        }
    }

    #[test]
    fn a_delay_that_runs_late_ends_the_check() {
        let mut io = TestIo::new();
        io.delay_extra = Duration::from_millis(5);
        let from = io.now();
        let check = block_on(scrambling_check(
            &mut io,
            from,
            Duration::from_millis(10),
            Duration::from_millis(3),
            &mut |_| {},
        ));
        // 0 ms, then 8 ms (3 + 5); another interval would reach 11 ms, past the window.
        assert_eq!(check.reads, 2);

        // A delay that itself runs past the window ends the check without its read.
        let mut io = TestIo::new();
        io.delay_extra = Duration::from_millis(8);
        let from = io.now();
        let check = block_on(scrambling_check(
            &mut io,
            from,
            Duration::from_millis(10),
            Duration::from_millis(3),
            &mut |_| {},
        ));
        // 0 ms, then a delay that ends at 11 ms.
        assert_eq!(check.reads, 1);
        assert!(io.ops().iter().any(|op| matches!(op, Op::Delay(_))));
    }

    #[test]
    fn a_zero_window_reads_once() {
        let mut io = TestIo::new();
        let from = io.now();
        let check = block_on(scrambling_check(
            &mut io,
            from,
            Duration::ZERO,
            INTERVAL,
            &mut |_| {},
        ));
        assert_eq!(check.reads, 1);
    }

    #[test]
    fn the_read_cap_follows_window_and_interval() {
        assert_eq!(read_cap(WINDOW, INTERVAL), 201);
        assert_eq!(
            read_cap(Duration::from_millis(10), Duration::from_millis(3)),
            5
        );
        assert_eq!(read_cap(Duration::ZERO, INTERVAL), 1);
        assert_eq!(read_cap(Duration::from_millis(1), Duration::ZERO), 1_001);
        assert_eq!(read_cap(Duration::MAX, Duration::ZERO), u32::MAX);
    }
}
