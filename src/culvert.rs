//! [`TmdsScdc`] for culvert's SCDC client (the `culvert` feature).
//!
//! Implemented for `culvert::Scdc<T>` itself, with no wrapper around the client, over any
//! `ScdcTransport`. Errors are culvert's, wrapped in [`CulvertError`] so that they carry a
//! [`DdcErrorKind`].

use culvert::{
    ClearableUpdateFlags, FfeLevels, FrlConfig, FrlRate, ReadModifyWriteError, Scdc, ScdcError,
    ScdcTransport, TmdsConfig,
};
use hdmi_hal::ddc::{DdcError, DdcErrorKind};

use crate::error::UpdateError;
use crate::scdc::TmdsScdc;
use crate::types::{ScramblingStatus, SinkTmdsConfig, TmdsConfigReading};

/// culvert's error, as [`TmdsScdc`] for `culvert::Scdc<T>` returns it.
///
/// A type of this crate's own, since [`DdcError`] cannot be implemented here for culvert's
/// type. Its [`kind`](DdcError::kind) is the transport's own for a transport failure, and
/// [`DdcErrorKind::Unsupported`] for a request culvert refuses as a protocol violation,
/// which would be refused again; the procedures' requests never produce one.
///
/// It is as `Copy` and `PartialEq` as culvert's error, so the procedures' outcomes stay so,
/// and a transparent wrapper otherwise: its `Display` is culvert's error's, and its
/// `source` the one culvert's error gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CulvertError<E>(pub ScdcError<E>);

impl<E: core::fmt::Display> core::fmt::Display for CulvertError<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

impl<E: core::error::Error + 'static> core::error::Error for CulvertError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.0.source()
    }
}

impl<E> From<ScdcError<E>> for CulvertError<E> {
    fn from(error: ScdcError<E>) -> Self {
        Self(error)
    }
}

impl<E: DdcError> DdcError for CulvertError<E> {
    fn kind(&self) -> DdcErrorKind {
        match &self.0 {
            ScdcError::Transport(e) => e.kind(),
            ScdcError::Protocol(_) => DdcErrorKind::Unsupported,
            _ => DdcErrorKind::Other,
        }
    }
}

fn config(config: SinkTmdsConfig) -> TmdsConfig {
    TmdsConfig {
        scrambling_enable: config.scrambling_enable,
        high_tmds_clock_ratio: config.high_tmds_clock_ratio,
    }
}

fn update_error<E>(error: ReadModifyWriteError<E>) -> UpdateError<CulvertError<E>> {
    match error {
        ReadModifyWriteError::Read(e) => UpdateError::Read(CulvertError(e)),
        ReadModifyWriteError::Write(e) => UpdateError::Write(CulvertError(e)),
    }
}

impl<T: ScdcTransport> TmdsScdc for Scdc<T> {
    type Error = CulvertError<T::Error>;

    fn write_tmds_config(&mut self, sink: SinkTmdsConfig) -> Result<(), Self::Error> {
        Ok(Scdc::write_tmds_config(self, config(sink))?)
    }

    fn update_tmds_config(&mut self, sink: SinkTmdsConfig) -> Result<(), UpdateError<Self::Error>> {
        Scdc::update_tmds_config(self, config(sink)).map_err(update_error)
    }

    fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, Self::Error> {
        let reading = Scdc::read_tmds_config(self)?;
        Ok(TmdsConfigReading::new(
            SinkTmdsConfig::new(
                reading.config.scrambling_enable,
                reading.config.high_tmds_clock_ratio,
            ),
            reading.raw,
        ))
    }

    fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, Self::Error> {
        let status = self.read_scrambler_status()?;
        Ok(ScramblingStatus::new(status.scrambling_active, status.raw))
    }

    fn read_sink_version(&mut self) -> Result<u8, Self::Error> {
        Ok(Scdc::read_sink_version(self)?)
    }

    fn write_source_version(&mut self, version: u8) -> Result<(), Self::Error> {
        Ok(Scdc::write_source_version(self, version)?)
    }

    fn clear_frl_rate(&mut self) -> Result<(), Self::Error> {
        // `Config_1` as 0: no FRL rate, FFE levels 0.
        Ok(self.write_frl_config(FrlConfig::new(FrlRate::NotSupported, FfeLevels::default()))?)
    }

    fn clear_flt_update(&mut self) -> Result<bool, UpdateError<Self::Error>> {
        let flags = self
            .read_update_flags()
            .map_err(|e| UpdateError::Read(CulvertError(e)))?;
        if flags.flt_update {
            let only_flt_update =
                ClearableUpdateFlags::new(false, false, false, false, true, false);
            self.clear_update_flags(only_flt_update)
                .map_err(|e| UpdateError::Write(CulvertError(e)))?;
        }
        Ok(flags.flt_update)
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use super::*;
    use hdmi_hal::ddc::NoAcknowledgeSource;

    /// An `ScdcTransport` over a register file, failing every transfer from `fail_from` on
    /// with a NACK. `Update_0` is write-1-to-clear, as on a sink.
    struct Sink {
        regs: [u8; 0x40],
        transfers: Cell<u32>,
        fail_from: u32,
    }

    impl Sink {
        fn new() -> Self {
            Self {
                regs: [0; 0x40],
                transfers: Cell::new(0),
                fail_from: u32::MAX,
            }
        }

        fn transfer(&self) -> Result<(), DdcErrorKind> {
            let n = self.transfers.get();
            self.transfers.set(n + 1);
            if n >= self.fail_from {
                Err(DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address))
            } else {
                Ok(())
            }
        }
    }

    impl ScdcTransport for Sink {
        type Error = DdcErrorKind;

        fn read(&self, reg: u8) -> Result<u8, DdcErrorKind> {
            self.transfer().map(|()| self.regs[usize::from(reg)])
        }

        fn write(&mut self, reg: u8, value: u8) -> Result<(), DdcErrorKind> {
            self.transfer()?;
            if reg == 0x10 {
                self.regs[0x10] &= !value;
            } else {
                self.regs[usize::from(reg)] = value;
            }
            Ok(())
        }

        fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), DdcErrorKind> {
            self.transfer()?;
            let start = usize::from(reg);
            buf.copy_from_slice(&self.regs[start..start + buf.len()]);
            Ok(())
        }
    }

    #[test]
    fn every_method_goes_through_culvert() {
        let mut sink = Sink::new();
        sink.regs[0x01] = 1;
        sink.regs[0x10] = 0x23;
        sink.regs[0x20] = 0xF0;
        sink.regs[0x21] = 0x81;
        sink.regs[0x31] = 0x53;
        let mut scdc = Scdc::new(sink);

        assert_eq!(TmdsScdc::read_sink_version(&mut scdc).unwrap(), 1);
        TmdsScdc::write_source_version(&mut scdc, 1).unwrap();
        TmdsScdc::clear_frl_rate(&mut scdc).unwrap();
        assert!(TmdsScdc::clear_flt_update(&mut scdc).unwrap());
        assert!(!TmdsScdc::clear_flt_update(&mut scdc).unwrap());

        TmdsScdc::update_tmds_config(&mut scdc, SinkTmdsConfig::new(true, true)).unwrap();
        let read = TmdsScdc::read_tmds_config(&mut scdc).unwrap();
        assert_eq!(read.config, SinkTmdsConfig::new(true, true));
        assert_eq!(read.raw, 0xF3);
        let status = TmdsScdc::read_scrambling_status(&mut scdc).unwrap();
        assert!(status.set);
        assert_eq!(status.raw(), 0x81);
        TmdsScdc::write_tmds_config(&mut scdc, SinkTmdsConfig::new(false, false)).unwrap();

        let sink = scdc.into_transport();
        assert_eq!(sink.regs[0x02], 1);
        assert_eq!(sink.regs[0x31], 0);
        assert_eq!(sink.regs[0x10], 0x03, "only FLT_update cleared");
        assert_eq!(sink.regs[0x20], 0x00);
    }

    #[test]
    fn errors_carry_the_transport_kind_and_say_which_transfer_failed() {
        let nack = DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address);
        let mut failing = Sink::new();
        failing.fail_from = 0;
        let mut scdc = Scdc::new(failing);
        assert_eq!(
            TmdsScdc::read_sink_version(&mut scdc).unwrap_err().kind(),
            nack
        );
        assert!(matches!(
            TmdsScdc::update_tmds_config(&mut scdc, SinkTmdsConfig::new(true, true)),
            Err(UpdateError::Read(e)) if e.kind() == nack
        ));
        assert!(matches!(
            TmdsScdc::clear_flt_update(&mut scdc),
            Err(UpdateError::Read(e)) if e.kind() == nack
        ));

        let mut after_read = Sink::new();
        after_read.regs[0x10] = 0x20;
        after_read.fail_from = 1;
        let mut scdc = Scdc::new(after_read);
        assert!(matches!(
            TmdsScdc::clear_flt_update(&mut scdc),
            Err(UpdateError::Write(e)) if e.kind() == nack
        ));
        let mut after_read = Sink::new();
        after_read.fail_from = 1;
        let mut scdc = Scdc::new(after_read);
        assert!(matches!(
            TmdsScdc::update_tmds_config(&mut scdc, SinkTmdsConfig::new(true, true)),
            Err(UpdateError::Write(e)) if e.kind() == nack
        ));
    }

    #[test]
    fn the_transport_reads_blocks_as_a_sink_would() {
        let mut sink = Sink::new();
        sink.regs[0x40 - 2] = 0xAB;
        sink.regs[0x40 - 1] = 0xCD;
        let mut buf = [0; 2];
        ScdcTransport::read_block(&sink, 0x40 - 2, &mut buf).unwrap();
        assert_eq!(buf, [0xAB, 0xCD]);
        sink.fail_from = 0;
        assert!(ScdcTransport::read_block(&sink, 0, &mut buf).is_err());
    }

    #[test]
    fn a_protocol_error_is_unsupported() {
        let refused = CulvertError::<DdcErrorKind>(ScdcError::Protocol(
            culvert::ProtocolError::UnknownFrlRate(0xF),
        ));
        assert_eq!(refused.kind(), DdcErrorKind::Unsupported);
        let wrapped: CulvertError<DdcErrorKind> = ScdcError::Transport(DdcErrorKind::Busy).into();
        assert_eq!(wrapped.kind(), DdcErrorKind::Busy);
    }

    #[test]
    fn the_wrapper_prints_compares_and_chains_as_culverts_error() {
        extern crate std;
        use core::error::Error;
        use std::string::ToString;

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct Bus;
        impl core::fmt::Display for Bus {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("bus gone")
            }
        }
        impl Error for Bus {}

        let inner = ScdcError::Transport(Bus);
        let wrapped = CulvertError(inner);
        assert_eq!(wrapped.to_string(), inner.to_string());
        assert_eq!(wrapped.source().unwrap().to_string(), "bus gone");
        let copied = wrapped;
        assert_eq!(copied, wrapped);

        // So the procedures' outcomes are Copy and comparable with the culvert feature too.
        fn copy_and_compare<T: Copy + PartialEq>() {}
        copy_and_compare::<crate::LinkUp<CulvertError<Bus>>>();
        copy_and_compare::<crate::BringUpError<CulvertError<Bus>, ()>>();
    }
}
