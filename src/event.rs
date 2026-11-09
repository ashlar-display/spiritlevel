//! What the procedures record, step by step: [`TmdsEvent`].
//!
//! Each procedure has a `_with_events` form taking `FnMut(TmdsEvent)`; the plain form passes
//! a recorder that does nothing. Events carry no error values, only their
//! [`DdcErrorKind`] (SCDC) or the step that failed (PHY): the values stay in the outcome and
//! the error, which own them. So an event is `Copy` with no type parameters, and an embedded
//! caller can send it through a channel. Each procedure emits at most a fixed number of
//! events, published as a `const` ([`MAX_TEAR_DOWN_EVENTS`]), so a caller without `alloc`
//! sizes its buffer and never drops one.

use core::time::Duration;

use hdmi_hal::ddc::DdcErrorKind;
use hdmi_hal::phy::TmdsVideo;
use hdmi_hal::time::Timestamp;

use crate::config::{BringUpConfig, SinkPresence, TearDownConfig};
use crate::outcome::{ReconnectCheck, SinkClear};
use crate::params::TmdsLinkParams;
use crate::types::ClockRatio;
use crate::types::{SinkTmdsConfig, SinkTmdsFacts, TmdsConfigReading};

/// The most events one bring-up records: its start; two each for the FRL clear and
/// `Source_Version`; one for stopping the output; two for the sink's `TMDS_Config` (the write
/// and a plain link's read-back); one for the PHY's configuration; two for the scrambler (off,
/// then on, when unscrambled output runs through the write); one each for the wait, the
/// output's start, the deadline, the scrambling check and a cancellation; the tear-down after
/// an error or a cancellation ([`MAX_TEAR_DOWN_EVENTS`]); and its end.
pub const MAX_BRING_UP_EVENTS: usize =
    1 + 2 + 2 + 1 + 2 + 1 + 2 + 1 + 1 + 1 + 1 + 1 + MAX_TEAR_DOWN_EVENTS + 1;

/// The most events one tear-down records: its start, one each for the sink, the scrambler
/// and the output, and its end.
pub const MAX_TEAR_DOWN_EVENTS: usize = 5;

/// The most events one reconnect check records: its start, the read (or its failure), and
/// its end.
pub const MAX_RECONNECT_CHECK_EVENTS: usize = 3;

/// The most events one quick restore records: its start, one each for the sink's write,
/// the scrambler and the scrambling check, and its end.
pub const MAX_QUICK_RESTORE_EVENTS: usize = 5;

/// One step or decision of a procedure, stamped by the procedure from its own clock.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TmdsEvent {
    /// When it happened.
    pub at: Timestamp,
    /// What happened.
    pub kind: TmdsEventKind,
}

impl TmdsEvent {
    /// An event of `kind` at `at`.
    #[must_use]
    pub const fn new(at: Timestamp, kind: TmdsEventKind) -> Self {
        Self { at, kind }
    }
}

/// What a procedure did, decided or saw.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmdsEventKind {
    /// A tear-down started.
    #[non_exhaustive]
    TearDownStarted {
        /// The running link it stops.
        link: TmdsLinkParams,
        /// Its settings.
        config: TearDownConfig,
        /// Whether the caller says the sink is still there.
        presence: SinkPresence,
    },
    /// The sink's `TMDS_Config` was written.
    #[non_exhaustive]
    TmdsConfigWritten {
        /// The value written.
        config: SinkTmdsConfig,
    },
    /// The sink's `TMDS_Config` was not cleared, and why.
    #[non_exhaustive]
    SinkClearSkipped {
        /// The rule that skipped it.
        reason: SinkClearSkip,
    },
    /// The source's scrambler was turned on or off.
    #[non_exhaustive]
    ScramblerSet {
        /// On or off.
        enabled: bool,
    },
    /// The output was stopped.
    OutputStopped,
    /// An SCDC step failed.
    #[non_exhaustive]
    ScdcStepFailed {
        /// The step.
        step: TmdsStep,
        /// The error's kind; the error itself is in the outcome or the error.
        kind: DdcErrorKind,
        /// Whether it ends the procedure; `false` for a warning the procedure carries past.
        ends: bool,
    },
    /// A PHY step failed. The error is in the outcome or the error: a PHY error has no kind.
    #[non_exhaustive]
    PhyStepFailed {
        /// The step.
        step: TmdsStep,
        /// Whether it ends the procedure; `false` when the procedure attempts its later
        /// steps anyway, as tear-down does.
        ends: bool,
    },
    /// A bring-up started.
    #[non_exhaustive]
    BringUpStarted {
        /// The link it brings up.
        link: TmdsLinkParams,
        /// The video the link carries.
        video: TmdsVideo,
        /// Its settings.
        config: BringUpConfig,
    },
    /// The sink's `TMDS_Config` was not written, and why.
    #[non_exhaustive]
    TmdsConfigSkipped {
        /// The rule that skipped it.
        reason: TmdsConfigSkip,
    },
    /// The PHY was configured.
    #[non_exhaustive]
    PhyConfigured {
        /// The TMDS character rate, in kHz.
        character_rate_khz: u32,
        /// The clock ratio.
        clock_ratio: ClockRatio,
    },
    /// The wait after the sink's write was spent: what was left of it once the steps since
    /// the write had run, possibly nothing.
    #[non_exhaustive]
    WaitSpent {
        /// The time waited.
        duration: Duration,
    },
    /// The output was started.
    #[non_exhaustive]
    OutputStarted {
        /// The time since the sink's write; `None` when it came before the write or no
        /// write was due.
        after_write: Option<Duration>,
    },
    /// Scrambled output reached the sink later after its write than the deadline allows.
    #[non_exhaustive]
    DeadlineMissed {
        /// By how much.
        by: Duration,
    },
    /// The scrambling check ended: one event per window, not per read.
    #[non_exhaustive]
    ScramblingChecked {
        /// What it found, with the first error's kind in place of the error.
        check: crate::outcome::ScramblingCheck<DdcErrorKind>,
    },
    /// A bring-up saw the caller's cancel flag and stopped; the tear-down's events follow,
    /// unless it was seen at entry.
    #[non_exhaustive]
    BringUpCancelled {
        /// Where the flag was seen.
        at: CancelPoint,
    },
    /// A bring-up ended.
    #[non_exhaustive]
    BringUpEnded {
        /// The step that stopped it; `None` when the link is up or the bring-up was
        /// cancelled.
        failed_at: Option<TmdsStep>,
        /// Whether the cancel flag stopped it.
        cancelled: bool,
    },
    /// The sink's `Config_1` (the FRL rate) was cleared before a bring-up.
    FrlRateCleared,
    /// The sink's `FLT_update` was cleared before a bring-up.
    #[non_exhaustive]
    FltUpdateCleared {
        /// Whether it was set.
        was_set: bool,
    },
    /// The FRL clear before a bring-up was skipped, and why.
    #[non_exhaustive]
    FrlClearSkipped {
        /// The rule that skipped it.
        reason: FrlClearSkip,
    },
    /// The sink's `Sink_Version` was read.
    #[non_exhaustive]
    SinkVersionRead {
        /// The value.
        version: u8,
    },
    /// `Source_Version` was written.
    #[non_exhaustive]
    SourceVersionWritten {
        /// The value.
        version: u8,
    },
    /// `Source_Version` was not written, and why.
    #[non_exhaustive]
    SourceVersionSkipped {
        /// The rule that skipped it.
        reason: SourceVersionSkip,
    },
    /// A reconnect check started.
    #[non_exhaustive]
    ReconnectCheckStarted {
        /// The running link.
        link: TmdsLinkParams,
        /// The sink's facts now, as the caller read them after the hotplug.
        current_sink: SinkTmdsFacts,
    },
    /// The sink's `TMDS_Config` was read.
    #[non_exhaustive]
    TmdsConfigRead {
        /// The register as read, with its byte.
        reading: TmdsConfigReading,
    },
    /// A reconnect check ended.
    #[non_exhaustive]
    ReconnectCheckEnded {
        /// What it found, with an error's kind in place of the error.
        result: ReconnectCheck<DdcErrorKind>,
    },
    /// A quick restore started.
    #[non_exhaustive]
    QuickRestoreStarted {
        /// The running link whose sink configuration it restores.
        link: TmdsLinkParams,
        /// The settings in force; the quick restore reads the `TMDS_Config` write's two and
        /// the check's timing.
        config: BringUpConfig,
    },
    /// A quick restore ended.
    #[non_exhaustive]
    QuickRestoreEnded {
        /// The step that stopped it; `None` when it ran to its end.
        failed_at: Option<TmdsStep>,
    },
    /// A tear-down ended: what became of each part, without the error values.
    #[non_exhaustive]
    TearDownEnded {
        /// The sink's clear, with an error's kind in place of the error.
        sink: SinkClear<DdcErrorKind>,
        /// `set_scrambling(false)` failed.
        scrambler_failed: bool,
        /// `stop_output` failed: the output may still run.
        output_failed: bool,
    },
}

/// A step of a procedure, as an event or an error names it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TmdsStep {
    /// Clearing the sink's `Config_1` (the FRL rate) before a bring-up.
    ClearFrlRate,
    /// Clearing the sink's `FLT_update` before a bring-up.
    ClearFltUpdate,
    /// Reading the sink's `Sink_Version`.
    SinkVersion,
    /// Writing `Source_Version`.
    SourceVersion,
    /// Writing the sink's `TMDS_Config`; for a read-modify-write, its write.
    WriteTmdsConfig,
    /// Reading the sink's `TMDS_Config`: a read-back, the reconnect check's read, or the
    /// read of a read-modify-write, after which nothing was written.
    ReadTmdsConfig,
    /// Configuring the PHY's character rate and clock ratio.
    Configure,
    /// Turning the source's scrambler on or off.
    SetScrambling,
    /// Starting the output.
    StartOutput,
    /// Stopping the output.
    StopOutput,
}

/// Where a bring-up saw the caller's cancel flag.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancelPoint {
    /// At entry, before any I/O.
    Entry,
    /// Before an SCDC step.
    Step(TmdsStep),
    /// During the ratio wait, before one of its slices.
    RatioWait,
    /// During the scrambling check, before a read.
    Check,
}

/// Why a bring-up did not write the sink's `TMDS_Config`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TmdsConfigSkip {
    /// The sink has no SCDC.
    NoScdc,
    /// A plain link on a sink the policy's `TmdsConfigGate` leaves unwritten.
    Gate,
}

/// Why the FRL clear before a bring-up was skipped.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrlClearSkip {
    /// The sink has no SCDC.
    NoScdc,
    /// The sink declares no FRL, and the setting clears only one that does.
    SinkDeclaresNoFrl,
}

/// Why `Source_Version` was not written.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceVersionSkip {
    /// The sink has no SCDC.
    NoScdc,
    /// The setting never writes it.
    Never,
    /// The setting writes it only on a scrambled link, and this one is not.
    NotScrambled,
    /// The sink's `Sink_Version` rules the write out under the setting: not 1 for
    /// `IfSinkVersion1`, 0 for `WhenScrambling`.
    #[non_exhaustive]
    SinkVersion {
        /// The value read.
        version: u8,
    },
    /// The `Sink_Version` read the setting needs failed, so nothing was written.
    SinkVersionUnread,
}

/// Why a tear-down did not clear the sink's `TMDS_Config`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SinkClearSkip {
    /// The settings do not clear it (`clear_sink: false`, the default).
    NotRequested,
    /// The link set nothing to clear: a plain link, or a sink without SCDC.
    NothingToClear,
    /// The caller says the sink is gone: no SCDC writes.
    SinkGone,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_copy<T: Copy>(_: T) {}

    #[test]
    fn events_are_copy_and_carry_no_error_values() {
        let event = TmdsEvent::new(
            Timestamp::from_micros(5),
            TmdsEventKind::ScdcStepFailed {
                step: TmdsStep::WriteTmdsConfig,
                kind: DdcErrorKind::Busy,
                ends: false,
            },
        );
        is_copy(event);
        let copy = event;
        assert_eq!(copy, event);
        assert_eq!(event.at, Timestamp::from_micros(5));
    }
}
