//! What the procedures record, step by step: [`TmdsEvent`].
//!
//! Each procedure has a `_with_events` form taking `FnMut(TmdsEvent)`; the plain form passes
//! a recorder that does nothing. Events carry no error values, only their
//! [`DdcErrorKind`] (SCDC) or the step that failed (PHY): the values stay in the outcome and
//! the error, which own them. So an event is `Copy` with no type parameters, and an embedded
//! caller can send it through a channel. Each procedure emits at most a fixed number of
//! events, published as a `const` ([`MAX_TEAR_DOWN_EVENTS`]), so a caller without `alloc`
//! sizes its buffer and never drops one.

use hdmi_hal::ddc::DdcErrorKind;
use hdmi_hal::time::Timestamp;

use crate::config::{SinkPresence, TearDownConfig};
use crate::outcome::{ReconnectCheck, SinkClear};
use crate::params::TmdsLinkParams;
use crate::types::{SinkTmdsConfig, SinkTmdsFacts};

/// The most events one tear-down records: its start, one each for the sink, the scrambler
/// and the output, and its end.
pub const MAX_TEAR_DOWN_EVENTS: usize = 5;

/// The most events one reconnect check records: its start, the read (or its failure), and
/// its end.
pub const MAX_RECONNECT_CHECK_EVENTS: usize = 3;

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
        config: SinkTmdsConfig,
    },
    /// A reconnect check ended.
    #[non_exhaustive]
    ReconnectChecked {
        /// What it found, with an error's kind in place of the error.
        result: ReconnectCheck<DdcErrorKind>,
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
    /// Writing the sink's `TMDS_Config`.
    WriteTmdsConfig,
    /// Reading the sink's `TMDS_Config`.
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
