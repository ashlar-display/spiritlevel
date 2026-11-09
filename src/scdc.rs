//! The sink-side operations the procedures make: [`TmdsScdc`].
//!
//! A trait of this crate's own rather than culvert's client: it holds the seven operations
//! the procedures need and nothing more, in the procedures' vocabulary ([`SinkTmdsConfig`],
//! [`ScramblingStatus`]), so a firmware with its own SCDC code, a kernel driver with SCDC
//! helpers, or a test double implements it without culvert's whole register map. With the
//! `culvert` feature, it is implemented for culvert's client.

use hdmi_hal::ddc::DdcError;

use crate::error::UpdateError;
use crate::types::{ScramblingStatus, SinkTmdsConfig, TmdsConfigReading};

/// The sink's SCDC registers, as the TMDS procedures use them.
///
/// | Procedure step | Call |
/// |---|---|
/// | Before bring-up: the FRL clear | [`clear_frl_rate`](Self::clear_frl_rate), [`clear_flt_update`](Self::clear_flt_update) |
/// | Before bring-up: `Source_Version` | [`read_sink_version`](Self::read_sink_version), [`write_source_version`](Self::write_source_version) |
/// | The sink's `TMDS_Config` | [`update_tmds_config`](Self::update_tmds_config) (default) or [`write_tmds_config`](Self::write_tmds_config) |
/// | Read-back, reconnect check | [`read_tmds_config`](Self::read_tmds_config) |
/// | The scrambling check | [`read_scrambling_status`](Self::read_scrambling_status) |
///
/// The bit layout of each register is the implementation's: it decodes what it reads into
/// [`TmdsConfigReading`] and [`ScramblingStatus`], keeping the byte.
///
/// # Errors
///
/// [`Error`](Self::Error) is a [`DdcError`]: its kind tells a failure that will not pass
/// (`Detached`, `Unsupported`) from one that may, so a procedure can poll through the second
/// and stop on the first. An implementation over an `ScdcTransport` passes the transport's
/// errors through, since `ScdcTransport::Error` carries the same bound. An operation made of
/// a read and a write returns [`UpdateError`], saying which of the two failed.
pub trait TmdsScdc {
    /// The error of one transfer, classified by [`DdcError::kind`].
    type Error: DdcError;

    /// Writes `TMDS_Config` (0x20) as a whole byte: the two bits as given, the others 0.
    fn write_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), Self::Error>;

    /// Reads `TMDS_Config` (0x20), sets the two bits from `config`, and writes the byte back
    /// with every other bit as read. A failed read writes nothing.
    ///
    /// # Errors
    ///
    /// [`UpdateError::Read`] when the read failed (nothing written), [`UpdateError::Write`]
    /// when the write failed.
    fn update_tmds_config(
        &mut self,
        config: SinkTmdsConfig,
    ) -> Result<(), UpdateError<Self::Error>>;

    /// Reads `TMDS_Config` (0x20), keeping the byte read ([`TmdsConfigReading::raw`]).
    fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, Self::Error>;

    /// Reads `Scrambling_Status` (0x21), keeping the byte read ([`ScramblingStatus::raw`]).
    fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, Self::Error>;

    /// Reads the sink's `Sink_Version` (0x01).
    fn read_sink_version(&mut self) -> Result<u8, Self::Error>;

    /// Writes `Source_Version` (0x02).
    fn write_source_version(&mut self, version: u8) -> Result<(), Self::Error>;

    /// Writes `Config_1` (0x31) as 0: `FRL_Rate` and `FFE_Levels` both, without reading it
    /// first.
    fn clear_frl_rate(&mut self) -> Result<(), Self::Error>;

    /// Reads `Update_0` (0x10) and, when `FLT_update` is set, writes it back as 1, which
    /// clears it; the other flags are left for whoever reads them next. Returns whether
    /// `FLT_update` was set.
    ///
    /// # Errors
    ///
    /// [`UpdateError::Read`] when the read failed (nothing known, nothing written),
    /// [`UpdateError::Write`] when the flag was set and the clear failed.
    fn clear_flt_update(&mut self) -> Result<bool, UpdateError<Self::Error>>;
}

/// The implementation behind a mutable reference, so a caller can lend its SCDC client to a
/// driver and keep it.
impl<T: TmdsScdc + ?Sized> TmdsScdc for &mut T {
    type Error = T::Error;

    fn write_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), Self::Error> {
        (**self).write_tmds_config(config)
    }

    fn update_tmds_config(
        &mut self,
        config: SinkTmdsConfig,
    ) -> Result<(), UpdateError<Self::Error>> {
        (**self).update_tmds_config(config)
    }

    fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, Self::Error> {
        (**self).read_tmds_config()
    }

    fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, Self::Error> {
        (**self).read_scrambling_status()
    }

    fn read_sink_version(&mut self) -> Result<u8, Self::Error> {
        (**self).read_sink_version()
    }

    fn write_source_version(&mut self, version: u8) -> Result<(), Self::Error> {
        (**self).write_source_version(version)
    }

    fn clear_frl_rate(&mut self) -> Result<(), Self::Error> {
        (**self).clear_frl_rate()
    }

    fn clear_flt_update(&mut self) -> Result<bool, UpdateError<Self::Error>> {
        (**self).clear_flt_update()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hdmi_hal::ddc::{DdcErrorKind, NoAcknowledgeSource};

    /// The registers the trait touches, as bytes, with bits 0 and 1 of `TMDS_Config` and
    /// bit 0 of `Scrambling_Status` as the layout; reads fail from `fail_from` on.
    struct Registers {
        regs: [u8; 0x40],
        transfers: u32,
        fail_from: u32,
    }

    impl Registers {
        fn new() -> Self {
            Self {
                regs: [0; 0x40],
                transfers: 0,
                fail_from: u32::MAX,
            }
        }

        fn transfer(&mut self) -> Result<(), DdcErrorKind> {
            self.transfers += 1;
            if self.transfers > self.fail_from {
                Err(DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Unknown))
            } else {
                Ok(())
            }
        }

        fn read(&mut self, reg: usize) -> Result<u8, DdcErrorKind> {
            self.transfer().map(|()| self.regs[reg])
        }

        fn write(&mut self, reg: usize, value: u8) -> Result<(), DdcErrorKind> {
            self.transfer().map(|()| self.regs[reg] = value)
        }
    }

    fn encode(config: SinkTmdsConfig) -> u8 {
        u8::from(config.scrambling_enable) | (u8::from(config.high_tmds_clock_ratio) << 1)
    }

    impl TmdsScdc for Registers {
        type Error = DdcErrorKind;

        fn write_tmds_config(&mut self, config: SinkTmdsConfig) -> Result<(), DdcErrorKind> {
            self.write(0x20, encode(config))
        }

        fn update_tmds_config(
            &mut self,
            config: SinkTmdsConfig,
        ) -> Result<(), UpdateError<DdcErrorKind>> {
            let byte = self.read(0x20).map_err(UpdateError::Read)?;
            self.write(0x20, (byte & !0x03) | encode(config))
                .map_err(UpdateError::Write)
        }

        fn read_tmds_config(&mut self) -> Result<TmdsConfigReading, DdcErrorKind> {
            let raw = self.read(0x20)?;
            Ok(TmdsConfigReading::new(
                SinkTmdsConfig::new(raw & 0x01 != 0, raw & 0x02 != 0),
                raw,
            ))
        }

        fn read_scrambling_status(&mut self) -> Result<ScramblingStatus, DdcErrorKind> {
            let raw = self.read(0x21)?;
            Ok(ScramblingStatus::new(raw & 0x01 != 0, raw))
        }

        fn read_sink_version(&mut self) -> Result<u8, DdcErrorKind> {
            self.read(0x01)
        }

        fn write_source_version(&mut self, version: u8) -> Result<(), DdcErrorKind> {
            self.write(0x02, version)
        }

        fn clear_frl_rate(&mut self) -> Result<(), DdcErrorKind> {
            self.write(0x31, 0)
        }

        fn clear_flt_update(&mut self) -> Result<bool, UpdateError<DdcErrorKind>> {
            let flags = self.read(0x10).map_err(UpdateError::Read)?;
            let set = flags & 0x20 != 0;
            if set {
                // Write-1-to-clear: only FLT_update; the sink keeps the other flags.
                self.write(0x10, 0x20).map_err(UpdateError::Write)?;
                self.regs[0x10] = flags & !0x20;
            }
            Ok(set)
        }
    }

    #[test]
    fn a_register_level_implementation_does_what_each_method_says() {
        let mut sink = Registers::new();
        sink.regs[0x01] = 1;
        sink.regs[0x20] = 0xF0;
        sink.regs[0x21] = 0x81;
        sink.regs[0x31] = 0x53;
        sink.regs[0x10] = 0x21;

        assert_eq!(sink.read_sink_version(), Ok(1));
        sink.write_source_version(1).unwrap();
        assert_eq!(sink.regs[0x02], 1);

        sink.clear_frl_rate().unwrap();
        assert_eq!(sink.regs[0x31], 0);
        assert_eq!(sink.clear_flt_update(), Ok(true));
        assert_eq!(sink.regs[0x10], 0x01, "Status_Update stays set");
        assert_eq!(sink.clear_flt_update(), Ok(false));

        sink.update_tmds_config(SinkTmdsConfig::new(true, true))
            .unwrap();
        assert_eq!(sink.regs[0x20], 0xF3, "reserved bits kept");
        let read = sink.read_tmds_config().unwrap();
        assert_eq!(read.config, SinkTmdsConfig::new(true, true));
        assert_eq!(read.raw, 0xF3);
        sink.write_tmds_config(SinkTmdsConfig::new(true, false))
            .unwrap();
        assert_eq!(sink.regs[0x20], 0x01, "whole byte written");

        let status = sink.read_scrambling_status().unwrap();
        assert!(status.set);
        assert_eq!(status.raw(), 0x81);
    }

    #[test]
    fn a_borrowed_implementation_forwards_every_call() {
        fn exercise<T: TmdsScdc<Error = DdcErrorKind>>(
            mut scdc: T,
        ) -> (u8, bool, TmdsConfigReading, ScramblingStatus) {
            let version = scdc.read_sink_version().unwrap();
            scdc.write_source_version(1).unwrap();
            scdc.clear_frl_rate().unwrap();
            let was_set = scdc.clear_flt_update().unwrap();
            scdc.update_tmds_config(SinkTmdsConfig::new(true, true))
                .unwrap();
            let read = scdc.read_tmds_config().unwrap();
            scdc.write_tmds_config(SinkTmdsConfig::new(false, false))
                .unwrap();
            (
                version,
                was_set,
                read,
                scdc.read_scrambling_status().unwrap(),
            )
        }
        let mut sink = Registers::new();
        sink.regs[0x01] = 1;
        sink.regs[0x21] = 0x01;
        sink.regs[0x31] = 0x53;
        sink.regs[0x10] = 0x20;
        let (version, was_set, read, status) = exercise(&mut sink);
        assert_eq!(
            (version, was_set, read.raw, status.set),
            (1, true, 0x03, true)
        );
        // The calls reached the lender's registers.
        assert_eq!(
            (
                sink.regs[0x02],
                sink.regs[0x31],
                sink.regs[0x10],
                sink.regs[0x20]
            ),
            (1, 0, 0, 0)
        );
    }

    #[test]
    fn read_and_write_failures_are_told_apart() {
        let mut sink = Registers::new();
        sink.regs[0x20] = 0x80;
        sink.fail_from = 0;
        assert!(matches!(
            sink.update_tmds_config(SinkTmdsConfig::new(true, true)),
            Err(UpdateError::Read(_))
        ));
        sink.transfers = 0;
        sink.fail_from = 1;
        assert!(matches!(
            sink.update_tmds_config(SinkTmdsConfig::new(true, true)),
            Err(UpdateError::Write(_))
        ));
        assert_eq!(sink.regs[0x20], 0x80, "nothing written");

        sink.regs[0x10] = 0x20;
        sink.transfers = 0;
        assert!(matches!(
            sink.clear_flt_update(),
            Err(UpdateError::Write(e)) if e.kind() == DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Unknown)
        ));
    }
}
