//! The sync driver: [`TmdsLink`] runs the procedures over a [`TmdsScdc`], a `TmdsPhy`, a
//! `Clock` and a `Delay`, with an optional [`Cancel`] flag.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;

use hdmi_hal::cancel::{Cancel, NoCancel};
use hdmi_hal::phy::{TmdsPhy, TmdsPhyConfig, TmdsVideo};
use hdmi_hal::time::{Clock, Delay, Timestamp};

use crate::config::{BringUpConfig, SinkPresence, TearDownConfig};
use crate::error::UpdateError;
use crate::event::TmdsEvent;
use crate::outcome::{
    BringUpError, BringUpOutcome, ReconnectCheck, RestoreError, Restored, TearDownError, TornDown,
};
use crate::params::TmdsLinkParams;
use crate::procedure::{self, TmdsIo};
use crate::scdc::TmdsScdc;
use crate::types::{ScramblingStatus, SinkTmdsConfig, SinkTmdsFacts, TmdsConfigReading};

/// What a tear-down returns, over the parts' error types.
type TearDownResult<S, P> = Result<
    TornDown<<S as TmdsScdc>::Error>,
    TearDownError<<S as TmdsScdc>::Error, <P as TmdsPhy>::Error>,
>;

/// What a bring-up returns, over the parts' error types.
type BringUpResult<S, P> = Result<
    BringUpOutcome<<S as TmdsScdc>::Error, <P as TmdsPhy>::Error>,
    BringUpError<<S as TmdsScdc>::Error, <P as TmdsPhy>::Error>,
>;

/// What a quick restore returns, over the parts' error types.
type RestoreResult<S, P> = Result<
    Restored<<S as TmdsScdc>::Error>,
    RestoreError<<S as TmdsScdc>::Error, <P as TmdsPhy>::Error>,
>;

/// The TMDS procedures over sync parts: the sink's SCDC, the PHY, a clock and a delay, and a
/// cancel flag ([`NoCancel`] unless [`with_cancel`](Self::with_cancel) gives one).
///
/// Each procedure runs to completion before it returns: the parts' calls complete at once,
/// and the delay blocks. One clock-and-delay value can serve as both: implement `Clock` and
/// `Delay` for a reference to it, and pass the reference twice.
#[derive(Debug)]
pub struct TmdsLink<S, P, K, D, X = NoCancel> {
    scdc: S,
    phy: P,
    clock: K,
    delay: D,
    cancel: X,
}

impl<S: TmdsScdc, P: TmdsPhy, K: Clock, D: Delay> TmdsLink<S, P, K, D, NoCancel> {
    /// The driver over its parts, never cancelled.
    pub fn new(scdc: S, phy: P, clock: K, delay: D) -> Self {
        Self {
            scdc,
            phy,
            clock,
            delay,
            cancel: NoCancel,
        }
    }
}

impl<S: TmdsScdc, P: TmdsPhy, K: Clock, D: Delay, X: Cancel> TmdsLink<S, P, K, D, X> {
    /// The same driver with `cancel` as its cancel flag.
    pub fn with_cancel<X2: Cancel>(self, cancel: X2) -> TmdsLink<S, P, K, D, X2> {
        TmdsLink {
            scdc: self.scdc,
            phy: self.phy,
            clock: self.clock,
            delay: self.delay,
            cancel,
        }
    }

    /// The parts back.
    pub fn into_parts(self) -> (S, P, K, D, X) {
        (self.scdc, self.phy, self.clock, self.delay, self.cancel)
    }

    /// The SCDC client.
    pub fn scdc(&self) -> &S {
        &self.scdc
    }

    /// The SCDC client, mutably: between procedures, for reads of the caller's own.
    pub fn scdc_mut(&mut self) -> &mut S {
        &mut self.scdc
    }

    /// The PHY.
    pub fn phy(&self) -> &P {
        &self.phy
    }

    /// The PHY, mutably: between procedures, to power it on or off, read its capabilities, or
    /// drive it in another mode.
    pub fn phy_mut(&mut self) -> &mut P {
        &mut self.phy
    }

    /// The clock the procedures stamp their events from, so a caller can stamp its own on
    /// the same timebase.
    pub fn clock(&self) -> &K {
        &self.clock
    }

    /// Stops the TMDS link `link` describes; see [`procedure::tear_down`].
    ///
    /// # Errors
    ///
    /// [`TearDownError`] when a PHY step failed; every step was still attempted.
    pub fn tear_down(
        &mut self,
        link: &TmdsLinkParams,
        config: &TearDownConfig,
        presence: SinkPresence,
    ) -> TearDownResult<S, P> {
        self.tear_down_with_events(link, config, presence, &mut |_| {})
    }

    /// [`tear_down`](Self::tear_down), calling `record` with each event.
    ///
    /// # Errors
    ///
    /// As [`tear_down`](Self::tear_down).
    pub fn tear_down_with_events(
        &mut self,
        link: &TmdsLinkParams,
        config: &TearDownConfig,
        presence: SinkPresence,
        record: &mut impl FnMut(TmdsEvent),
    ) -> TearDownResult<S, P> {
        block_on(procedure::tear_down(
            &mut SyncIo(self),
            link,
            config,
            presence,
            record,
        ))
    }
}

impl<S: TmdsScdc, P: TmdsPhy, K: Clock, D: Delay, X: Cancel> TmdsLink<S, P, K, D, X> {
    /// Brings up the TMDS link `link` describes, carrying `video`; see
    /// [`procedure::bring_up`].
    ///
    /// # Errors
    ///
    /// [`BringUpError`] when a step failed in a way that stops the bring-up.
    pub fn bring_up(
        &mut self,
        link: &TmdsLinkParams,
        video: TmdsVideo,
        config: &BringUpConfig,
    ) -> BringUpResult<S, P> {
        self.bring_up_with_events(link, video, config, &mut |_| {})
    }

    /// [`bring_up`](Self::bring_up), calling `record` with each event.
    ///
    /// # Errors
    ///
    /// As [`bring_up`](Self::bring_up).
    pub fn bring_up_with_events(
        &mut self,
        link: &TmdsLinkParams,
        video: TmdsVideo,
        config: &BringUpConfig,
        record: &mut impl FnMut(TmdsEvent),
    ) -> BringUpResult<S, P> {
        block_on(procedure::bring_up(
            &mut SyncIo(self),
            link,
            video,
            config,
            record,
        ))
    }

    /// Checks whether the sink still holds the `TMDS_Config` `link` needs; see
    /// [`procedure::check_reconnect`].
    pub fn check_reconnect(
        &mut self,
        link: &TmdsLinkParams,
        current_sink: SinkTmdsFacts,
    ) -> ReconnectCheck<S::Error> {
        self.check_reconnect_with_events(link, current_sink, &mut |_| {})
    }

    /// [`check_reconnect`](Self::check_reconnect), calling `record` with each event.
    pub fn check_reconnect_with_events(
        &mut self,
        link: &TmdsLinkParams,
        current_sink: SinkTmdsFacts,
        record: &mut impl FnMut(TmdsEvent),
    ) -> ReconnectCheck<S::Error> {
        block_on(procedure::check_reconnect(
            &mut SyncIo(self),
            link,
            current_sink,
            record,
        ))
    }

    /// Restores the sink's `TMDS_Config` on the running link; see
    /// [`procedure::quick_restore`].
    ///
    /// # Errors
    ///
    /// [`RestoreError`] when the sink's write or the scrambler call failed in a way that
    /// stops it.
    pub fn quick_restore(
        &mut self,
        link: &TmdsLinkParams,
        config: &BringUpConfig,
    ) -> RestoreResult<S, P> {
        self.quick_restore_with_events(link, config, &mut |_| {})
    }

    /// [`quick_restore`](Self::quick_restore), calling `record` with each event.
    ///
    /// # Errors
    ///
    /// As [`quick_restore`](Self::quick_restore).
    pub fn quick_restore_with_events(
        &mut self,
        link: &TmdsLinkParams,
        config: &BringUpConfig,
        record: &mut impl FnMut(TmdsEvent),
    ) -> RestoreResult<S, P> {
        block_on(procedure::quick_restore(
            &mut SyncIo(self),
            link,
            config,
            record,
        ))
    }
}

/// Polls `future` once: the sync parts' calls complete at once, so it is ready.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => unreachable!("spiritlevel's sync driver waited on I/O"),
    }
}

/// [`TmdsIo`] over a [`TmdsLink`]'s parts: each operation is the sync call itself.
struct SyncIo<'a, S, P, K, D, X>(&'a mut TmdsLink<S, P, K, D, X>);

impl<S: TmdsScdc, P: TmdsPhy, K: Clock, D: Delay, X: Cancel> TmdsIo for SyncIo<'_, S, P, K, D, X> {
    type ScdcError = S::Error;
    type PhyError = P::Error;

    async fn write_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), S::Error> {
        self.0.scdc.write_tmds_config(config)
    }

    async fn update_tmds_config(
        &mut self,
        config: SinkTmdsConfig,
    ) -> Result<(), UpdateError<S::Error>> {
        self.0.scdc.update_tmds_config(config)
    }

    async fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, S::Error> {
        self.0.scdc.read_tmds_config()
    }

    async fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, S::Error> {
        self.0.scdc.read_scrambling_status()
    }

    async fn read_sink_version(&mut self) -> Result<u8, S::Error> {
        self.0.scdc.read_sink_version()
    }

    async fn write_source_version(&mut self, version: u8) -> Result<(), S::Error> {
        self.0.scdc.write_source_version(version)
    }

    async fn clear_frl_rate(&mut self) -> Result<(), S::Error> {
        self.0.scdc.clear_frl_rate()
    }

    async fn clear_flt_update(&mut self) -> Result<bool, UpdateError<S::Error>> {
        self.0.scdc.clear_flt_update()
    }

    async fn configure(&mut self, config: TmdsPhyConfig) -> Result<(), P::Error> {
        self.0.phy.configure(config)
    }

    async fn set_scrambling(&mut self, enabled: bool) -> Result<(), P::Error> {
        self.0.phy.set_scrambling(enabled)
    }

    async fn start_output(&mut self) -> Result<(), P::Error> {
        self.0.phy.start_output()
    }

    async fn stop_output(&mut self) -> Result<(), P::Error> {
        self.0.phy.stop_output()
    }

    fn now(&self) -> Timestamp {
        self.0.clock.now()
    }

    async fn delay(&mut self, duration: Duration) {
        self.0.delay.delay(duration);
    }

    fn cancelled(&self) -> bool {
        self.0.cancel.is_cancelled()
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use hdmi_hal::ddc::DdcErrorKind;
    use hdmi_hal::phy::TmdsPhyCapabilities;

    use core::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::outcome::SinkClear;
    use crate::types::{SinkTmdsFacts, SourceTmdsFacts, TmdsPolicy};

    /// The sink's registers that matter here: `TMDS_Config`, the version registers and the
    /// FRL clear's.
    #[derive(Default)]
    struct Sink {
        tmds_config: u8,
        sink_version: u8,
        source_version: u8,
        flt_update: bool,
        frl_clears: u32,
        writes: u32,
    }

    impl TmdsScdc for Sink {
        type Error = DdcErrorKind;

        fn write_tmds_config(&mut self, c: SinkTmdsConfig) -> Result<(), DdcErrorKind> {
            self.writes += 1;
            self.tmds_config =
                u8::from(c.scrambling_enable) | (u8::from(c.high_tmds_clock_ratio) << 1);
            Ok(())
        }

        fn update_tmds_config(
            &mut self,
            c: SinkTmdsConfig,
        ) -> Result<(), UpdateError<DdcErrorKind>> {
            self.write_tmds_config(c).map_err(UpdateError::Write)
        }

        fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, DdcErrorKind> {
            let raw = self.tmds_config;
            Ok(TmdsConfigReading::new(
                SinkTmdsConfig::new(raw & 1 != 0, raw & 2 != 0),
                raw,
            ))
        }

        fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, DdcErrorKind> {
            Ok(ScramblingStatus::new(true, 1))
        }

        fn read_sink_version(&mut self) -> Result<u8, DdcErrorKind> {
            Ok(self.sink_version)
        }

        fn write_source_version(&mut self, version: u8) -> Result<(), DdcErrorKind> {
            self.source_version = version;
            Ok(())
        }

        fn clear_frl_rate(&mut self) -> Result<(), DdcErrorKind> {
            self.frl_clears += 1;
            Ok(())
        }

        fn clear_flt_update(&mut self) -> Result<bool, UpdateError<DdcErrorKind>> {
            Ok(core::mem::take(&mut self.flt_update))
        }
    }

    /// A PHY that tracks whether it transmits and scrambles.
    #[derive(Default)]
    struct Phy {
        output: bool,
        scrambling: bool,
        configured: Option<TmdsPhyConfig>,
    }

    impl TmdsPhy for Phy {
        type Error = ();

        fn capabilities(&self) -> TmdsPhyCapabilities {
            TmdsPhyCapabilities::new(600_000).with_scrambling(true)
        }

        fn configure(&mut self, config: TmdsPhyConfig) -> Result<(), ()> {
            self.configured = Some(config);
            Ok(())
        }

        fn set_scrambling(&mut self, enabled: bool) -> Result<(), ()> {
            self.scrambling = enabled;
            Ok(())
        }

        fn start_output(&mut self) -> Result<(), ()> {
            self.output = true;
            Ok(())
        }

        fn stop_output(&mut self) -> Result<(), ()> {
            self.output = false;
            Ok(())
        }
    }

    /// One time source as both clock and delay, shared by reference.
    #[derive(Default)]
    struct SimTime {
        now: Cell<u64>,
    }

    impl Clock for &SimTime {
        fn now(&self) -> Timestamp {
            Timestamp::from_micros(self.now.get())
        }
    }

    impl Delay for &SimTime {
        fn delay(&mut self, duration: Duration) {
            let micros = u64::try_from(duration.as_micros()).unwrap();
            self.now.set(self.now.get() + micros);
        }
    }

    fn scrambled() -> TmdsLinkParams {
        TmdsLinkParams::choose(
            594_000,
            SinkTmdsFacts::new(true, true, false),
            SourceTmdsFacts::from(&Phy::default().capabilities()),
            TmdsPolicy::DEFAULT,
        )
        .unwrap()
    }

    #[test]
    fn tear_down_runs_over_the_parts() {
        let time = SimTime::default();
        let sink = Sink {
            tmds_config: 0x03,
            ..Sink::default()
        };
        let phy = Phy {
            output: true,
            scrambling: true,
            ..Phy::default()
        };
        let mut link = TmdsLink::new(sink, phy, &time, &time);

        let mut events = 0;
        let result = link.tear_down_with_events(
            &scrambled(),
            &TearDownConfig::DEFAULT.with_clear_sink(true),
            SinkPresence::Present,
            &mut |_| events += 1,
        );
        assert_eq!(
            result,
            Ok(TornDown {
                sink: SinkClear::Cleared
            })
        );
        assert_eq!(events, crate::MAX_TEAR_DOWN_EVENTS);

        let (sink, phy, ..) = link.into_parts();
        assert_eq!(sink.tmds_config, 0);
        assert!(!phy.output && !phy.scrambling);
        assert_eq!(phy.configured, None);
    }

    #[test]
    fn bring_up_runs_over_the_parts() {
        let time = SimTime::default();
        let mut link = TmdsLink::new(Sink::default(), Phy::default(), &time, &time);
        let params = scrambled();
        let video = hdmi_hal::phy::TmdsVideo::new(
            594_000,
            hdmi_hal::ColorBitDepth::Depth8,
            hdmi_hal::ColorFormat::Rgb444,
        );
        let config = BringUpConfig::DEFAULT;
        assert!(matches!(
            link.bring_up(&params, video, &config),
            Ok(BringUpOutcome::Up(_))
        ));
        let mut events = 0;
        assert!(
            link.bring_up_with_events(&params, video, &config, &mut |_| events += 1)
                .is_ok()
        );
        assert!(events > 0 && events <= crate::MAX_BRING_UP_EVENTS);
        let (sink, phy, ..) = link.into_parts();
        assert_eq!(sink.tmds_config, 0x03);
        assert_eq!(sink.source_version, 1);
        assert!(phy.output && phy.scrambling);
        assert_eq!(
            phy.configured.map(|c| c.clock_ratio),
            Some(crate::ClockRatio::Div40)
        );
    }

    #[test]
    fn check_reconnect_runs_over_the_parts() {
        let time = SimTime::default();
        let sink = Sink {
            tmds_config: 0x03,
            ..Sink::default()
        };
        let mut link = TmdsLink::new(sink, Phy::default(), &time, &time);
        let params = scrambled();
        assert_eq!(
            link.check_reconnect(&params, params.sink()),
            ReconnectCheck::Matches
        );
        let mut events = 0;
        let result =
            link.check_reconnect_with_events(&params, SinkTmdsFacts::NONE, &mut |_| events += 1);
        assert_eq!(result, ReconnectCheck::SinkChanged);
        assert_eq!(events, 2);
    }

    #[test]
    fn quick_restore_runs_over_the_parts() {
        let time = SimTime::default();
        let mut link = TmdsLink::new(Sink::default(), Phy::default(), &time, &time);
        let restored = link
            .quick_restore(&scrambled(), &BringUpConfig::DEFAULT)
            .unwrap();
        assert_eq!(restored.check.result, crate::CheckResult::Set);
        let mut events = 0;
        assert!(
            link.quick_restore_with_events(&scrambled(), &BringUpConfig::DEFAULT, &mut |_| {
                events += 1;
            })
            .is_ok()
        );
        assert_eq!(events, crate::MAX_QUICK_RESTORE_EVENTS);
        let (sink, phy, ..) = link.into_parts();
        assert_eq!(sink.tmds_config, 0x03);
        assert!(phy.scrambling);
    }

    #[test]
    fn the_parts_are_reachable_between_procedures() {
        let time = SimTime::default();
        let mut link = TmdsLink::new(Sink::default(), Phy::default(), &time, &time);
        link.scdc_mut().source_version = 7;
        link.phy_mut().output = true;
        assert_eq!(link.scdc().source_version, 7);
        assert!(link.phy().output);
        assert_eq!(link.clock().now(), Timestamp::from_micros(0));
    }

    #[test]
    fn the_plain_form_and_a_cancel_flag() {
        let time = SimTime::default();
        let flag = AtomicBool::new(false);
        let mut link =
            TmdsLink::new(Sink::default(), Phy::default(), &time, &time).with_cancel(&flag);
        assert_eq!(
            link.tear_down(&scrambled(), &TearDownConfig::DEFAULT, SinkPresence::Gone),
            Ok(TornDown {
                sink: SinkClear::NotRequested
            })
        );
        // A flag that is not set lets a bring-up through.
        let video = hdmi_hal::phy::TmdsVideo::new(
            594_000,
            hdmi_hal::ColorBitDepth::Depth8,
            hdmi_hal::ColorFormat::Rgb444,
        );
        assert!(matches!(
            link.bring_up(&scrambled(), video, &BringUpConfig::DEFAULT),
            Ok(BringUpOutcome::Up(_))
        ));
        // Tear-down is the recovery: a set flag does not stop it.
        flag.store(true, Ordering::Relaxed);
        assert!(
            link.tear_down(&scrambled(), &TearDownConfig::DEFAULT, SinkPresence::Gone)
                .is_ok()
        );
        // A bring-up sees it at entry and makes no call.
        let video = hdmi_hal::phy::TmdsVideo::new(
            594_000,
            hdmi_hal::ColorBitDepth::Depth8,
            hdmi_hal::ColorFormat::Rgb444,
        );
        assert!(matches!(
            link.bring_up(&scrambled(), video, &BringUpConfig::DEFAULT),
            Ok(BringUpOutcome::Cancelled {
                at: crate::CancelPoint::Entry,
                cleanup: crate::Cleanup::Skipped,
                ..
            })
        ));
        let (sink, _, _, _, cancel) = link.into_parts();
        // The first bring-up's write; the cancelled one made none.
        assert_eq!(sink.writes, 1);
        assert!(cancel.is_cancelled());
        assert!(!NoCancel.is_cancelled());
    }

    #[test]
    fn the_sync_io_passes_every_call_through() {
        let time = SimTime::default();
        let sink = Sink {
            sink_version: 2,
            flt_update: true,
            ..Sink::default()
        };
        let mut link = TmdsLink::new(sink, Phy::default(), &time, &time);
        let mut io = SyncIo(&mut link);
        let video = hdmi_hal::phy::TmdsVideo::new(
            148_500,
            hdmi_hal::ColorBitDepth::Depth8,
            hdmi_hal::ColorFormat::Rgb444,
        );
        block_on(async {
            io.update_tmds_config(SinkTmdsConfig::new(true, false))
                .await
                .unwrap();
            assert_eq!(io.read_tmds_config().await.unwrap().raw, 1);
            assert!(io.read_scrambling_status().await.unwrap().set);
            assert_eq!(io.read_sink_version().await, Ok(2));
            io.write_source_version(1).await.unwrap();
            io.clear_frl_rate().await.unwrap();
            assert_eq!(io.clear_flt_update().await, Ok(true));
            assert_eq!(io.clear_flt_update().await, Ok(false));
            io.configure(TmdsPhyConfig::new(148_500, crate::ClockRatio::Div10, video))
                .await
                .unwrap();
            io.start_output().await.unwrap();
            let before = io.now();
            io.delay(Duration::from_millis(1)).await;
            assert_eq!(
                io.now().checked_duration_since(before),
                Some(Duration::from_millis(1))
            );
            assert!(!io.cancelled());
        });
        let (sink, phy, ..) = link.into_parts();
        assert_eq!((sink.source_version, sink.frl_clears), (1, 1));
        assert!(phy.output && phy.configured.is_some());
    }
}
