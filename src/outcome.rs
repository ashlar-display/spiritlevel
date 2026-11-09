//! What the procedures return: for bring-up, [`BringUpOutcome`] or [`BringUpError`]; for
//! tear-down, [`TornDown`] or [`TearDownError`], with [`SinkClear`] saying what became of the
//! sink; for the reconnect check, [`ReconnectCheck`]; for the quick restore, [`Restored`] or
//! [`RestoreError`].
//!
//! `S` is the SCDC error ([`TmdsScdc::Error`](crate::TmdsScdc::Error)), `P` the PHY error.

use core::fmt;
use core::time::Duration;

use hdmi_hal::ddc::{DdcError, DdcErrorKind};
use hdmi_hal::time::Timestamp;

use crate::error::UpdateError;
use crate::event::{CancelPoint, TmdsStep};
use crate::params::TmdsLinkParams;
use crate::types::TmdsConfigReading;

/// How a bring-up ended without an error.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BringUpOutcome<S, P> {
    /// The link is up.
    Up(LinkUp<S>),
    /// The caller's cancel flag stopped the attempt. Never a failure.
    #[non_exhaustive]
    Cancelled {
        /// Where the flag was seen.
        at: CancelPoint,
        /// The tear-down that followed, whatever `tear_down_on_error` says;
        /// [`Cleanup::Skipped`] for a flag seen at entry, before any I/O.
        cleanup: Cleanup<S, P>,
        /// What went wrong before the flag was seen, without stopping the bring-up.
        warnings: Warnings<S>,
    },
}

/// A link that is up: steps up to the output's start completed. Whether the sink reports
/// scrambling is the check's, part of the outcome rather than a condition of it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkUp<S> {
    /// The link brought up.
    pub link: TmdsLinkParams,
    /// When the sink's `TMDS_Config` was written, or a write that failed was made (it may
    /// have reached the sink); `None` when no write was due or the read of a read-modify-write
    /// failed, so nothing was written.
    pub sink_written_at: Option<Timestamp>,
    /// When the output started.
    pub output_started_at: Timestamp,
    /// The scrambling check; [`CheckResult::NotApplicable`] on a plain link. Never
    /// [`CheckResult::Cancelled`]: a cancelled check ends the bring-up as
    /// [`BringUpOutcome::Cancelled`].
    pub check: ScramblingCheck<S>,
    /// What went wrong without stopping the bring-up.
    pub warnings: Warnings<S>,
}

/// What went wrong without stopping a bring-up. Each kind occurs at most once per attempt.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BringUpWarning<S> {
    /// Clearing `Config_1` failed.
    FrlRateClearFailed(S),
    /// Clearing `FLT_update` failed, at its read or its write.
    FltUpdateClearFailed(UpdateError<S>),
    /// Reading `Sink_Version` failed.
    SinkVersionReadFailed(S),
    /// Writing `Source_Version` failed.
    SourceVersionWriteFailed(S),
    /// Writing the sink's `TMDS_Config` failed.
    #[non_exhaustive]
    TmdsConfigWriteFailed {
        /// The error, and which half of the write failed: the read of a read-modify-write
        /// (nothing was written) or the write (always the write for a whole-byte write).
        error: UpdateError<S>,
        /// On a plain link, the register read back after the failure, or the read's error;
        /// `None` on other links, which the check covers, and when the cancel flag stopped
        /// the bring-up before the read.
        read_back: Option<Result<TmdsConfigReading, S>>,
    },
    /// Scrambled output reached the sink later after its write than the deadline allows: the
    /// output's start, or with `Running` the scrambler coming on.
    #[non_exhaustive]
    DeadlineMissed {
        /// By how much.
        by: Duration,
    },
    /// The scrambling check did not see the sink report scrambled TMDS: the link is up, but
    /// the sink may show nothing. [`LinkUp::check`] has the details.
    #[non_exhaustive]
    ScramblingNotConfirmed {
        /// [`CheckResult::NotSet`] or [`CheckResult::Unreadable`].
        result: CheckResult,
    },
}

/// How many warnings a bring-up can gather: one of each kind.
const WARNING_KINDS: usize = 7;

/// The warnings a bring-up gathered, in the order they arose. Fixed storage, so no
/// `alloc`, and never full: each kind occurs at most once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Warnings<S> {
    list: [Option<BringUpWarning<S>>; WARNING_KINDS],
    len: usize,
}

impl<S> Warnings<S> {
    /// No warnings.
    pub(crate) const fn new() -> Self {
        Self {
            list: [None, None, None, None, None, None, None],
            len: 0,
        }
    }

    /// Adds `warning`. A bring-up adds each kind at most once, so there is always room.
    pub(crate) fn push(&mut self, warning: BringUpWarning<S>) {
        if let Some(slot) = self.list.get_mut(self.len) {
            *slot = Some(warning);
            self.len += 1;
        }
    }

    /// The warnings, in the order they arose.
    pub fn iter_warnings(&self) -> impl Iterator<Item = &BringUpWarning<S>> {
        self.list[..self.len].iter().flatten()
    }

    /// How many there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Why a bring-up stopped.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BringUpError<S, P> {
    /// The sink's `TMDS_Config` write failed with a non-transient error (`Detached`,
    /// `Unsupported`), or at all under `OnTmdsConfigFailure::Stop`. The work before both
    /// paths and a plain link's read-back only warn.
    #[non_exhaustive]
    Scdc {
        /// The step that failed.
        step: TmdsStep,
        /// The error.
        error: S,
        /// The output's state before the cleanup.
        output: OutputState,
        /// What happened after the error.
        cleanup: Cleanup<S, P>,
        /// The warnings gathered before it.
        warnings: Warnings<S>,
    },
    /// A PHY call failed.
    #[non_exhaustive]
    Phy {
        /// The step that failed.
        step: TmdsStep,
        /// The error.
        error: P,
        /// The output's state before the cleanup.
        output: OutputState,
        /// What happened after the error.
        cleanup: Cleanup<S, P>,
        /// The warnings gathered before it.
        warnings: Warnings<S>,
    },
}

/// What happened after a bring-up's error or cancellation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cleanup<S, P> {
    /// A tear-down ran (after an error with `tear_down_on_error`, the default, or after a
    /// cancellation); its result.
    TornDown(Result<TornDown<S>, TearDownError<S, P>>),
    /// No tear-down: after an error with `tear_down_on_error: false`, both ends as the
    /// error left them; or a cancellation seen at entry, before any I/O.
    Skipped,
}

/// Whether the output may be running. Exhaustive: it is stopped or it may not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputState {
    /// Stopped.
    Stopped,
    /// May be running: started, or a call that would have started or stopped it failed.
    MayBeRunning,
}

/// What the scrambling check found, and how.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScramblingCheck<S> {
    /// How it ended.
    pub result: CheckResult,
    /// Reads made.
    pub reads: u32,
    /// Reads that failed.
    pub failed_reads: u32,
    /// The first failed read.
    pub first_error: Option<FailedRead<S>>,
    /// When the sink was first seen reporting scrambling, from the check's start: the
    /// output's start, or the scrambler coming on (with `Running`, and in the quick
    /// restore).
    pub set_after: Option<Duration>,
    /// The check ended on its read count, not its window: the clock did not advance as
    /// the reads expected.
    pub read_cap_reached: bool,
}

impl<S> ScramblingCheck<S> {
    /// The check of a plain link: nothing read.
    pub(crate) const NOT_APPLICABLE: Self = Self {
        result: CheckResult::NotApplicable,
        reads: 0,
        failed_reads: 0,
        first_error: None,
        set_after: None,
        read_cap_reached: false,
    };
}

impl<S: DdcError> ScramblingCheck<S> {
    /// The same record with the first error's kind in place of the error, as an event
    /// carries it.
    #[must_use]
    pub fn kinds(&self) -> ScramblingCheck<DdcErrorKind> {
        ScramblingCheck {
            result: self.result,
            reads: self.reads,
            failed_reads: self.failed_reads,
            first_error: self.first_error.as_ref().map(|f| FailedRead {
                error: f.error.kind(),
                after: f.after,
            }),
            set_after: self.set_after,
            read_cap_reached: self.read_cap_reached,
        }
    }
}

/// How the scrambling check ended.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckResult {
    /// A plain link: nothing to check.
    NotApplicable,
    /// The sink reports scrambled TMDS.
    Set,
    /// The sink did not report it within the window; the link is left running.
    NotSet,
    /// Every read failed, or one failed with a non-transient error; the link is left
    /// running.
    Unreadable,
    /// The cancel flag stopped it.
    Cancelled,
}

/// A failed read of the check, and when it came.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailedRead<S> {
    /// The error.
    pub error: S,
    /// When it came, from the check's start (see [`ScramblingCheck::set_after`]).
    pub after: Duration,
}

/// What became of the sink's `TMDS_Config` in a tear-down.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkClear<S> {
    /// The settings do not clear it (`clear_sink: false`, the default).
    NotRequested,
    /// The link set nothing to clear: a plain link, or a sink without SCDC.
    NothingToClear,
    /// The caller said the sink is gone, so nothing was written.
    SkippedSinkGone,
    /// It was cleared.
    Cleared,
    /// The write failed: a warning, as tear-down went on.
    Failed(S),
}

impl<S: DdcError> SinkClear<S> {
    /// The same result with the error's kind in place of the error, as an event carries it.
    #[must_use]
    pub fn kinds(&self) -> SinkClear<DdcErrorKind> {
        match self {
            Self::NotRequested => SinkClear::NotRequested,
            Self::NothingToClear => SinkClear::NothingToClear,
            Self::SkippedSinkGone => SinkClear::SkippedSinkGone,
            Self::Cleared => SinkClear::Cleared,
            Self::Failed(e) => SinkClear::Failed(e.kind()),
        }
    }
}

/// What the reconnect check found.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectCheck<S> {
    /// The sink's `TMDS_Config` holds what the link needs.
    Matches,
    /// The sink lost its `TMDS_Config`: it holds `read`, not what the link needs. The way
    /// back is the caller's: a tear-down and a bring-up, or the quick restore.
    #[non_exhaustive]
    Lost {
        /// The register as read, with its byte.
        read: TmdsConfigReading,
    },
    /// The read failed; nothing was decided.
    Unreadable(S),
    /// No `TMDS_Config` write is due on this link ([`TmdsLinkParams::sink_config`] is
    /// `None`: a sink without SCDC, or a plain link the policy's gate leaves unwritten), so
    /// there is nothing to check.
    NotApplicable,
    /// The sink's current facts differ from those the link was chosen with: nothing was
    /// read, as the link no longer fits the sink.
    SinkChanged,
}

impl<S: DdcError> ReconnectCheck<S> {
    /// The same result with the error's kind in place of the error, as an event carries it.
    #[must_use]
    pub fn kinds(&self) -> ReconnectCheck<DdcErrorKind> {
        match self {
            Self::Matches => ReconnectCheck::Matches,
            Self::Lost { read } => ReconnectCheck::Lost { read: *read },
            Self::Unreadable(e) => ReconnectCheck::Unreadable(e.kind()),
            Self::NotApplicable => ReconnectCheck::NotApplicable,
            Self::SinkChanged => ReconnectCheck::SinkChanged,
        }
    }
}

/// A quick restore that ran to its end.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Restored<S> {
    /// When the sink's `TMDS_Config` was written, or a write that failed was made; `None`
    /// when no write was due or the read of a read-modify-write failed.
    pub sink_written_at: Option<Timestamp>,
    /// The scrambling check; [`CheckResult::NotApplicable`] on a plain link, and
    /// [`CheckResult::Cancelled`] when the cancel flag stopped it, with the link left
    /// running.
    pub check: ScramblingCheck<S>,
    /// The write failed and the restore carried on
    /// ([`OnTmdsConfigFailure::CarryOn`](crate::OnTmdsConfigFailure::CarryOn)): its error,
    /// with the half of a read-modify-write that failed.
    pub write_failed: Option<UpdateError<S>>,
}

/// A quick restore that stopped. Nothing is torn down: the link was running.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreError<S, P> {
    /// The sink's write failed with a non-transient error, or at all under
    /// [`OnTmdsConfigFailure::Stop`](crate::OnTmdsConfigFailure::Stop); nothing followed.
    Scdc(UpdateError<S>),
    /// Setting the source's scrambler failed after the sink's write; its state is unknown.
    #[non_exhaustive]
    Phy {
        /// The error.
        error: P,
        /// When the sink's `TMDS_Config` was written, or a write that failed was made;
        /// `None` when no write was due or the read of a read-modify-write failed.
        sink_written_at: Option<Timestamp>,
        /// The write's failure, when it failed and the restore carried on.
        write_failed: Option<UpdateError<S>>,
    },
}

/// A tear-down in which every PHY step succeeded: the output is stopped.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TornDown<S> {
    /// What became of the sink.
    pub sink: SinkClear<S>,
}

/// A tear-down in which at least one PHY step failed. Every step was still attempted, so a
/// failed scrambler call does not leave the output running; each field holds the first
/// failure of its step.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TearDownError<S, P> {
    /// What became of the sink.
    pub sink: SinkClear<S>,
    /// `set_scrambling(false)` failed with this error.
    pub scrambler: Option<P>,
    /// `stop_output` failed with this error; `None`: the output is stopped.
    pub output: Option<P>,
}

impl<S: fmt::Display, P: fmt::Debug> fmt::Display for BringUpError<S, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scdc { step, error, .. } => {
                write!(f, "bring-up stopped at {step:?}: SCDC error: {error}")
            }
            Self::Phy { step, error, .. } => {
                write!(f, "bring-up stopped at {step:?}: PHY error: {error:?}")
            }
        }
    }
}

/// The SCDC error is the source; a PHY error, which is only `Debug`, has none.
impl<S: core::error::Error + 'static, P: fmt::Debug> core::error::Error for BringUpError<S, P> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Scdc { error, .. } => Some(error),
            Self::Phy { .. } => None,
        }
    }
}

impl<S, P: fmt::Debug> fmt::Display for TearDownError<S, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tear-down: ")?;
        match (&self.scrambler, &self.output) {
            (Some(s), Some(o)) => write!(
                f,
                "turning the scrambler off failed: {s:?}; stopping the output failed: {o:?}"
            ),
            (Some(s), None) => write!(f, "turning the scrambler off failed: {s:?}"),
            (None, Some(o)) => write!(f, "stopping the output failed: {o:?}"),
            (None, None) => f.write_str("a PHY step failed"),
        }
    }
}

/// Its failures are the PHY's, which are only `Debug`, so it has no source.
impl<S: fmt::Debug, P: fmt::Debug> core::error::Error for TearDownError<S, P> {}

impl<S: fmt::Display, P: fmt::Debug> fmt::Display for RestoreError<S, P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scdc(error) => write!(f, "quick restore stopped at the sink's write: {error}"),
            Self::Phy { error, .. } => {
                write!(f, "quick restore: setting the scrambler failed: {error:?}")
            }
        }
    }
}

/// The write's error is the source; a PHY error, which is only `Debug`, has none.
impl<S: core::error::Error + 'static, P: fmt::Debug> core::error::Error for RestoreError<S, P> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Scdc(error) => Some(error),
            Self::Phy { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Bus;

    impl fmt::Display for Bus {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("bus gone")
        }
    }

    impl core::error::Error for Bus {}

    #[test]
    fn errors_print_and_chain() {
        extern crate std;
        use core::error::Error;
        use std::string::ToString;

        let scdc = BringUpError::<Bus, ()>::Scdc {
            step: TmdsStep::WriteTmdsConfig,
            error: Bus,
            output: OutputState::Stopped,
            cleanup: Cleanup::Skipped,
            warnings: Warnings::new(),
        };
        assert_eq!(
            scdc.to_string(),
            "bring-up stopped at WriteTmdsConfig: SCDC error: bus gone"
        );
        assert_eq!(scdc.source().unwrap().to_string(), "bus gone");
        let phy = BringUpError::<Bus, ()>::Phy {
            step: TmdsStep::StartOutput,
            error: (),
            output: OutputState::MayBeRunning,
            cleanup: Cleanup::Skipped,
            warnings: Warnings::new(),
        };
        assert_eq!(
            phy.to_string(),
            "bring-up stopped at StartOutput: PHY error: ()"
        );
        assert!(phy.source().is_none());

        let torn = |scrambler, output| TearDownError::<Bus, ()> {
            sink: SinkClear::NotRequested,
            scrambler,
            output,
        };
        assert_eq!(
            torn(Some(()), Some(())).to_string(),
            "tear-down: turning the scrambler off failed: (); stopping the output failed: ()"
        );
        assert_eq!(
            torn(Some(()), None).to_string(),
            "tear-down: turning the scrambler off failed: ()"
        );
        assert_eq!(
            torn(None, Some(())).to_string(),
            "tear-down: stopping the output failed: ()"
        );
        assert_eq!(torn(None, None).to_string(), "tear-down: a PHY step failed");
        assert!(torn(None, Some(())).source().is_none());

        let restore = RestoreError::<Bus, ()>::Scdc(UpdateError::Read(Bus));
        assert_eq!(
            restore.to_string(),
            "quick restore stopped at the sink's write: read failed, nothing written: bus gone"
        );
        assert!(restore.source().is_some());
        let restore = RestoreError::<Bus, ()>::Phy {
            error: (),
            sink_written_at: None,
            write_failed: None,
        };
        assert_eq!(
            restore.to_string(),
            "quick restore: setting the scrambler failed: ()"
        );
        assert!(restore.source().is_none());
    }

    #[test]
    fn warnings_keep_the_order_and_never_overflow() {
        let mut w = Warnings::<DdcErrorKind>::new();
        assert!(w.is_empty());
        w.push(BringUpWarning::SourceVersionWriteFailed(DdcErrorKind::Busy));
        w.push(BringUpWarning::DeadlineMissed {
            by: Duration::from_millis(3),
        });
        assert_eq!(w.len(), 2);
        let mut it = w.iter_warnings();
        assert_eq!(
            it.next(),
            Some(&BringUpWarning::SourceVersionWriteFailed(
                DdcErrorKind::Busy
            ))
        );
        assert!(matches!(
            it.next(),
            Some(BringUpWarning::DeadlineMissed { .. })
        ));
        assert_eq!(it.next(), None);
        drop(it);
        // An eighth push has nowhere to go and is dropped rather than panicking; a
        // bring-up never makes one.
        for _ in 0..6 {
            w.push(BringUpWarning::FrlRateClearFailed(DdcErrorKind::Other));
        }
        assert_eq!(w.len(), WARNING_KINDS);
    }

    #[test]
    fn a_plain_links_check_read_nothing() {
        let check = ScramblingCheck::<DdcErrorKind>::NOT_APPLICABLE;
        assert_eq!(check.result, CheckResult::NotApplicable);
        assert_eq!((check.reads, check.failed_reads), (0, 0));
        assert!(check.first_error.is_none() && check.set_after.is_none());
        assert!(!check.read_cap_reached);
    }

    #[test]
    fn reconnect_kinds_keep_the_result_and_drop_the_value() {
        let read = TmdsConfigReading::new(crate::types::SinkTmdsConfig::new(false, false), 0x80);
        assert_eq!(
            ReconnectCheck::<DdcErrorKind>::Unreadable(DdcErrorKind::Detached).kinds(),
            ReconnectCheck::Unreadable(DdcErrorKind::Detached)
        );
        for check in [
            ReconnectCheck::<DdcErrorKind>::Matches,
            ReconnectCheck::Lost { read },
            ReconnectCheck::NotApplicable,
            ReconnectCheck::SinkChanged,
        ] {
            assert_eq!(check.kinds(), check);
        }
    }

    #[test]
    fn kinds_keeps_the_result_and_drops_the_value() {
        assert_eq!(
            SinkClear::<DdcErrorKind>::Failed(DdcErrorKind::Busy).kinds(),
            SinkClear::Failed(DdcErrorKind::Busy)
        );
        for clear in [
            SinkClear::<DdcErrorKind>::NotRequested,
            SinkClear::NothingToClear,
            SinkClear::SkippedSinkGone,
            SinkClear::Cleared,
        ] {
            assert_eq!(clear.kinds(), clear);
        }
    }
}
