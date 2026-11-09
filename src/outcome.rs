//! What the procedures return: for bring-up, [`BringUpOutcome`] or [`BringUpError`]; for
//! tear-down, [`TornDown`] or [`TearDownError`], with [`SinkClear`] saying what became of the
//! sink; for the reconnect check, [`ReconnectCheck`].
//!
//! `S` is the SCDC error ([`TmdsScdc::Error`](crate::TmdsScdc::Error)), `P` the PHY error.

use core::time::Duration;

use hdmi_hal::ddc::{DdcError, DdcErrorKind};
use hdmi_hal::time::Timestamp;

use crate::event::{CancelPoint, TmdsStep};
use crate::params::TmdsLinkParams;
use crate::types::SinkTmdsConfig;

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
    /// When the sink's `TMDS_Config` was written; `None` when no write was due.
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
    /// Clearing `FLT_update` failed.
    FltUpdateClearFailed(S),
    /// Reading `Sink_Version` failed.
    SinkVersionReadFailed(S),
    /// Writing `Source_Version` failed.
    SourceVersionWriteFailed(S),
    /// Writing the sink's `TMDS_Config` failed.
    #[non_exhaustive]
    TmdsConfigWriteFailed {
        /// The error.
        error: S,
        /// Which half of the write failed: the read of a read-modify-write (nothing was
        /// written) or the write ([`WriteStage::Write`] for a whole-byte write).
        stage: WriteStage,
        /// On a plain link, the register read back after the failure, or the read's error;
        /// `None` on other links, which the check covers, and when the cancel flag stopped
        /// the bring-up before the read.
        read_back: Option<Result<SinkTmdsConfig, S>>,
    },
    /// The output started later after the sink's write than the deadline allows.
    #[non_exhaustive]
    DeadlineMissed {
        /// By how much.
        by: Duration,
    },
}

/// How many warnings a bring-up can gather: one of each kind.
const WARNING_KINDS: usize = 6;

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
            list: [None, None, None, None, None, None],
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
    pub fn iter(&self) -> impl Iterator<Item = &BringUpWarning<S>> {
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
    /// A sink write or read failed with a non-transient error (`Detached`, `Unsupported`),
    /// or a `TMDS_Config` write failed under `OnTmdsConfigFailure::Stop`.
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

/// Which half of a `TMDS_Config` write failed. Exhaustive: a write has these two halves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WriteStage {
    /// The read of a read-modify-write: nothing was written.
    Read,
    /// The write.
    Write,
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
    /// When the sink was first seen reporting scrambling, from the output's start.
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
    /// When it came, from the output's start.
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
    Lost {
        /// The register as read, with its byte.
        read: crate::types::SinkTmdsConfig,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warnings_keep_the_order_and_never_overflow() {
        let mut w = Warnings::<DdcErrorKind>::new();
        assert!(w.is_empty());
        w.push(BringUpWarning::SourceVersionWriteFailed(DdcErrorKind::Busy));
        w.push(BringUpWarning::DeadlineMissed {
            by: Duration::from_millis(3),
        });
        assert_eq!(w.len(), 2);
        let mut it = w.iter();
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
        // A seventh push has nowhere to go and is dropped rather than panicking; a
        // bring-up never makes one.
        for _ in 0..5 {
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
        let read = crate::types::SinkTmdsConfig::from_read(false, false, 0x80);
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
