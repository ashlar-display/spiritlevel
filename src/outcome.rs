//! What the procedures return: for tear-down, [`TornDown`] or [`TearDownError`], with
//! [`SinkClear`] saying what became of the sink.
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
