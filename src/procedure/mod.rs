//! The procedures' I/O-free core: [`TmdsIo`], the I/O they need, and the procedures over it,
//! written once as `async fn`.
//!
//! A driver implements [`TmdsIo`] over its parts (a [`TmdsScdc`](crate::TmdsScdc), a
//! `TmdsPhy`, a `Clock` and a `Delay`) and polls the procedure's future: a sync driver whose
//! I/O completes at once polls it to completion without a runtime, an async driver awaits
//! it. A test drives the core through one double.

mod bring_up;
mod reconnect;
mod tear_down;

pub use reconnect::check_reconnect;
pub use tear_down::tear_down;

use core::time::Duration;

use hdmi_hal::ddc::DdcError;
use hdmi_hal::phy::TmdsPhyConfig;
use hdmi_hal::time::Timestamp;

use crate::error::UpdateError;
use crate::types::{ScramblingStatus, SinkTmdsConfig};

/// The I/O a procedure makes, flattened into one trait: the sink's SCDC registers as
/// [`TmdsScdc`](crate::TmdsScdc) has them, the PHY's four I/O methods as `TmdsPhy` has them,
/// the clock, the delay, and the cancel flag.
///
/// Each method's contract is its namesake's.
#[allow(async_fn_in_trait)]
pub trait TmdsIo {
    /// The SCDC error, classified by [`DdcError::kind`].
    type ScdcError: DdcError;
    /// The PHY error.
    type PhyError: core::fmt::Debug;

    /// See [`TmdsScdc::write_tmds_config`](crate::TmdsScdc::write_tmds_config).
    async fn write_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), Self::ScdcError>;
    /// See [`TmdsScdc::update_tmds_config`](crate::TmdsScdc::update_tmds_config).
    async fn update_tmds_config(
        &mut self,
        config: SinkTmdsConfig,
    ) -> Result<(), UpdateError<Self::ScdcError>>;
    /// See [`TmdsScdc::read_tmds_config`](crate::TmdsScdc::read_tmds_config).
    async fn read_tmds_config(&mut self) -> Result<SinkTmdsConfig, Self::ScdcError>;
    /// See [`TmdsScdc::read_scrambling_status`](crate::TmdsScdc::read_scrambling_status).
    async fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, Self::ScdcError>;
    /// See [`TmdsScdc::read_sink_version`](crate::TmdsScdc::read_sink_version).
    async fn read_sink_version(&mut self) -> Result<u8, Self::ScdcError>;
    /// See [`TmdsScdc::write_source_version`](crate::TmdsScdc::write_source_version).
    async fn write_source_version(&mut self, version: u8) -> Result<(), Self::ScdcError>;
    /// See [`TmdsScdc::clear_frl_rate`](crate::TmdsScdc::clear_frl_rate).
    async fn clear_frl_rate(&mut self) -> Result<(), Self::ScdcError>;
    /// See [`TmdsScdc::clear_flt_update`](crate::TmdsScdc::clear_flt_update).
    async fn clear_flt_update(&mut self) -> Result<bool, UpdateError<Self::ScdcError>>;

    /// See `TmdsPhy::configure`.
    async fn configure(&mut self, config: TmdsPhyConfig) -> Result<(), Self::PhyError>;
    /// See `TmdsPhy::set_scrambling`.
    async fn set_scrambling(&mut self, enabled: bool) -> Result<(), Self::PhyError>;
    /// See `TmdsPhy::start_output`.
    async fn start_output(&mut self) -> Result<(), Self::PhyError>;
    /// See `TmdsPhy::stop_output`.
    async fn stop_output(&mut self) -> Result<(), Self::PhyError>;

    /// The current time, from the platform's clock (`Clock::now`).
    fn now(&self) -> Timestamp;
    /// Waits at least `duration` (`Delay::delay`).
    async fn delay(&mut self, duration: Duration);
    /// Whether the caller has asked the procedure to stop.
    fn cancelled(&self) -> bool;
}

#[cfg(test)]
pub(crate) mod test_io;
