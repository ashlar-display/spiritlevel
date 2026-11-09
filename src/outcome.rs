//! What the procedures return: for tear-down, [`TornDown`] or [`TearDownError`], with
//! [`SinkClear`] saying what became of the sink; for the reconnect check,
//! [`ReconnectCheck`].
//!
//! `S` is the SCDC error ([`TmdsScdc::Error`](crate::TmdsScdc::Error)), `P` the PHY error.

use hdmi_hal::ddc::{DdcError, DdcErrorKind};

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
    /// The sink has no SCDC: there is nothing to check.
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
