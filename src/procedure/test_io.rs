//! A [`TmdsIo`] double for the procedures' tests: it records every operation, answers from
//! a few registers, fails what it is told to, and advances its clock by one microsecond on
//! every read and by the full duration on every delay.

use core::cell::Cell;
use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;

use hdmi_hal::ddc::DdcErrorKind;
use hdmi_hal::phy::TmdsPhyConfig;
use hdmi_hal::time::Timestamp;

use super::TmdsIo;
use crate::error::UpdateError;
use crate::types::{ScramblingStatus, SinkTmdsConfig, TmdsConfigReading};

/// Polls `future` to completion. The double's I/O completes at once, so one poll does.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("the double's I/O never waits"),
    }
}

/// One operation the double received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    WriteTmdsConfig(SinkTmdsConfig),
    UpdateTmdsConfig(SinkTmdsConfig),
    ReadTmdsConfig,
    ReadScramblingStatus,
    ReadSinkVersion,
    WriteSourceVersion(u8),
    ClearFrlRate,
    ClearFltUpdate,
    Configure(TmdsPhyConfig),
    SetScrambling(bool),
    StartOutput,
    StopOutput,
    Delay(Duration),
}

const LOG_LEN: usize = 512;

pub struct TestIo {
    log: [Op; LOG_LEN],
    len: usize,
    clock: Cell<u64>,
    /// SCDC calls fail with this kind, once `scdc_ok_calls` of them have succeeded.
    pub fail_scdc: Option<DdcErrorKind>,
    pub scdc_ok_calls: usize,
    /// How many SCDC calls fail after that; the rest succeed again.
    pub scdc_failures: usize,
    scdc_calls: usize,
    /// A failing `update_tmds_config` fails at its write rather than its read.
    pub update_fails_at_write: bool,
    pub fail_set_scrambling: bool,
    pub fail_stop_output: bool,
    pub fail_configure: bool,
    pub fail_start_output: bool,
    /// `TMDS_Config`, `Scrambling_Status`, `Sink_Version`, and whether `FLT_update` is set.
    pub tmds_config: u8,
    pub scrambling_status: u8,
    pub sink_version: u8,
    pub flt_update: bool,
    pub cancelled: bool,
    /// The cancel flag reads as set once this many operations were made.
    pub cancel_after_ops: Option<usize>,
    /// `Scrambling_Status` reads as set from this read on (1-based), overriding the register.
    pub status_set_from_read: Option<u32>,
    status_reads: u32,
    /// The clock does not advance: neither on a read nor on a delay.
    pub frozen_clock: bool,
}

impl TestIo {
    pub fn new() -> Self {
        Self {
            log: [Op::StopOutput; LOG_LEN],
            len: 0,
            clock: Cell::new(0),
            fail_scdc: None,
            scdc_ok_calls: 0,
            scdc_failures: usize::MAX,
            scdc_calls: 0,
            update_fails_at_write: false,
            fail_set_scrambling: false,
            fail_stop_output: false,
            fail_configure: false,
            fail_start_output: false,
            tmds_config: 0,
            scrambling_status: 0,
            sink_version: 1,
            flt_update: false,
            cancelled: false,
            cancel_after_ops: None,
            status_set_from_read: None,
            status_reads: 0,
            frozen_clock: false,
        }
    }

    /// The operations so far, in order.
    pub fn ops(&self) -> &[Op] {
        &self.log[..self.len]
    }

    fn push(&mut self, op: Op) {
        self.log[self.len] = op;
        self.len += 1;
    }

    fn scdc(&mut self, op: Op) -> Result<(), DdcErrorKind> {
        self.push(op);
        self.scdc_calls += 1;
        match self.fail_scdc {
            Some(kind)
                if self.scdc_calls > self.scdc_ok_calls
                    && self.scdc_calls - self.scdc_ok_calls <= self.scdc_failures =>
            {
                Err(kind)
            }
            _ => Ok(()),
        }
    }

    fn phy(&mut self, op: Op, fail: bool) -> Result<(), ()> {
        self.push(op);
        if fail { Err(()) } else { Ok(()) }
    }
}

impl TmdsIo for TestIo {
    type ScdcError = DdcErrorKind;
    type PhyError = ();

    async fn write_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), DdcErrorKind> {
        self.scdc(Op::WriteTmdsConfig(config))
    }

    async fn update_tmds_config(
        &mut self,
        config: SinkTmdsConfig,
    ) -> Result<(), UpdateError<DdcErrorKind>> {
        let at_write = self.update_fails_at_write;
        self.scdc(Op::UpdateTmdsConfig(config)).map_err(|e| {
            if at_write {
                UpdateError::Write(e)
            } else {
                UpdateError::Read(e)
            }
        })
    }

    async fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, DdcErrorKind> {
        self.scdc(Op::ReadTmdsConfig)?;
        let raw = self.tmds_config;
        Ok(TmdsConfigReading::new(
            SinkTmdsConfig::new(raw & 1 != 0, raw & 2 != 0),
            raw,
        ))
    }

    async fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, DdcErrorKind> {
        self.scdc(Op::ReadScramblingStatus)?;
        self.status_reads += 1;
        let raw = match self.status_set_from_read {
            Some(from) if self.status_reads >= from => 0x01,
            Some(_) => 0x00,
            None => self.scrambling_status,
        };
        Ok(ScramblingStatus::new(raw & 1 != 0, raw))
    }

    async fn read_sink_version(&mut self) -> Result<u8, DdcErrorKind> {
        self.scdc(Op::ReadSinkVersion).map(|()| self.sink_version)
    }

    async fn write_source_version(&mut self, version: u8) -> Result<(), DdcErrorKind> {
        self.scdc(Op::WriteSourceVersion(version))
    }

    async fn clear_frl_rate(&mut self) -> Result<(), DdcErrorKind> {
        self.scdc(Op::ClearFrlRate)
    }

    async fn clear_flt_update(&mut self) -> Result<bool, UpdateError<DdcErrorKind>> {
        self.scdc(Op::ClearFltUpdate).map_err(UpdateError::Read)?;
        Ok(core::mem::take(&mut self.flt_update))
    }

    async fn configure(&mut self, config: TmdsPhyConfig) -> Result<(), ()> {
        let fail = self.fail_configure;
        self.phy(Op::Configure(config), fail)
    }

    async fn set_scrambling(&mut self, enabled: bool) -> Result<(), ()> {
        let fail = self.fail_set_scrambling;
        self.phy(Op::SetScrambling(enabled), fail)
    }

    async fn start_output(&mut self) -> Result<(), ()> {
        let fail = self.fail_start_output;
        self.phy(Op::StartOutput, fail)
    }

    async fn stop_output(&mut self) -> Result<(), ()> {
        let fail = self.fail_stop_output;
        self.phy(Op::StopOutput, fail)
    }

    fn now(&self) -> Timestamp {
        if !self.frozen_clock {
            self.clock.set(self.clock.get() + 1);
        }
        Timestamp::from_micros(self.clock.get())
    }

    async fn delay(&mut self, duration: Duration) {
        self.push(Op::Delay(duration));
        if !self.frozen_clock {
            let micros = u64::try_from(duration.as_micros()).unwrap();
            self.clock.set(self.clock.get() + micros);
        }
    }

    fn cancelled(&self) -> bool {
        self.cancelled || self.cancel_after_ops.is_some_and(|n| self.len >= n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClockRatio;
    use hdmi_hal::phy::TmdsVideo;

    #[test]
    fn the_double_records_and_answers() {
        let mut io = TestIo::new();
        io.tmds_config = 0x03;
        io.scrambling_status = 0x01;
        io.flt_update = true;
        let video = TmdsVideo::new(
            594_000,
            hdmi_hal::ColorBitDepth::Depth8,
            hdmi_hal::ColorFormat::Rgb444,
        );
        let phy = TmdsPhyConfig::new(594_000, ClockRatio::Div40, video);
        block_on(async {
            io.write_tmds_config(SinkTmdsConfig::new(true, true))
                .await
                .unwrap();
            io.update_tmds_config(SinkTmdsConfig::new(true, true))
                .await
                .unwrap();
            assert_eq!(io.read_tmds_config().await.unwrap().raw, 0x03);
            assert!(io.read_scrambling_status().await.unwrap().set);
            assert_eq!(io.read_sink_version().await, Ok(1));
            io.write_source_version(1).await.unwrap();
            io.clear_frl_rate().await.unwrap();
            assert_eq!(io.clear_flt_update().await, Ok(true));
            io.configure(phy).await.unwrap();
            io.set_scrambling(true).await.unwrap();
            io.start_output().await.unwrap();
            io.stop_output().await.unwrap();
            let before = io.now();
            io.delay(Duration::from_millis(1)).await;
            assert!(io.now().checked_duration_since(before).unwrap() >= Duration::from_millis(1));
        });
        assert_eq!(io.ops().len(), 13);
        assert!(!io.cancelled());

        io.fail_scdc = Some(DdcErrorKind::Busy);
        io.fail_configure = true;
        io.fail_start_output = true;
        block_on(async {
            assert_eq!(
                io.update_tmds_config(SinkTmdsConfig::new(false, false))
                    .await,
                Err(UpdateError::Read(DdcErrorKind::Busy))
            );
            assert_eq!(
                io.clear_flt_update().await,
                Err(UpdateError::Read(DdcErrorKind::Busy))
            );
            assert_eq!(io.read_tmds_config().await.unwrap_err(), DdcErrorKind::Busy);
            assert_eq!(
                io.read_scrambling_status().await.unwrap_err(),
                DdcErrorKind::Busy
            );
            assert_eq!(io.configure(phy).await, Err(()));
            assert_eq!(io.start_output().await, Err(()));
        });
    }
}
