//! The errors: `ParamsError`, which `TmdsLinkParams::choose` returns, and `UpdateError`,
//! for an SCDC operation made of a read and a write.

use core::fmt;

use hdmi_hal::ddc::{DdcError, DdcErrorKind};

/// Why [`TmdsLinkParams::choose`](crate::TmdsLinkParams::choose) refused the inputs.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamsError {
    /// The rate is above 340 000 kHz, so the link must be scrambled, but the sink cannot
    /// take scrambling or the source cannot scramble. Every missing fact is set; at least
    /// one is.
    #[non_exhaustive]
    ScramblingUnsupported {
        /// The TMDS character rate asked for, in kHz.
        rate_khz: u32,
        /// The sink has no SCDC, so its `TMDS_Config` cannot be written.
        no_scdc: bool,
        /// The sink does not support scrambling
        /// ([`SinkTmdsFacts::scrambling_supported`](crate::SinkTmdsFacts::scrambling_supported)
        /// is clear).
        no_scrambling_support: bool,
        /// The source has no TMDS scrambler
        /// ([`SourceTmdsFacts::scrambling`](crate::SourceTmdsFacts::scrambling) is clear).
        source_cannot_scramble: bool,
    },
    /// The rate is above the source's own highest TMDS character rate
    /// ([`SourceTmdsFacts::max_character_rate_khz`](crate::SourceTmdsFacts::max_character_rate_khz)).
    #[non_exhaustive]
    AboveSourceMaximum {
        /// The TMDS character rate asked for, in kHz.
        rate_khz: u32,
        /// The source's highest TMDS character rate, in kHz.
        max_khz: u32,
    },
}

impl fmt::Display for ParamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ScramblingUnsupported {
                rate_khz,
                no_scdc,
                no_scrambling_support,
                source_cannot_scramble,
            } => {
                write!(
                    f,
                    "TMDS character rate {rate_khz} kHz needs scrambling, but"
                )?;
                let sink = match (no_scdc, no_scrambling_support) {
                    (true, true) => Some(" the sink has no SCDC and does not support scrambling"),
                    (true, false) => Some(" the sink has no SCDC"),
                    (false, true) => Some(" the sink does not support scrambling"),
                    (false, false) => None,
                };
                match (sink, source_cannot_scramble) {
                    (Some(sink), true) => write!(f, "{sink}, and the source has no scrambler"),
                    (Some(sink), false) => f.write_str(sink),
                    (None, true) => f.write_str(" the source has no scrambler"),
                    (None, false) => f.write_str(" the sink cannot take it"),
                }
            }
            Self::AboveSourceMaximum { rate_khz, max_khz } => write!(
                f,
                "TMDS character rate {rate_khz} kHz is above the source's maximum of {max_khz} kHz"
            ),
        }
    }
}

impl core::error::Error for ParamsError {}

/// The error of an SCDC operation made of a read and a write, saying which of them failed:
/// a read-modify-write of `TMDS_Config`, or the clear of `FLT_update`.
///
/// Exhaustive: such an operation is a read and a write, never a third transfer. Its
/// [`DdcError`] kind is that of the half that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdateError<E> {
    /// The read failed; nothing was written.
    Read(E),
    /// The read succeeded and the write failed; whether the sink took the write is unknown.
    Write(E),
}

impl<E> UpdateError<E> {
    /// The underlying error, whichever transfer it came from.
    pub fn into_inner(self) -> E {
        match self {
            Self::Read(e) | Self::Write(e) => e,
        }
    }
}

impl<E: DdcError> DdcError for UpdateError<E> {
    fn kind(&self) -> DdcErrorKind {
        match self {
            Self::Read(e) | Self::Write(e) => e.kind(),
        }
    }
}

impl<E: fmt::Display> fmt::Display for UpdateError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "read failed, nothing written: {e}"),
            Self::Write(e) => write!(f, "write failed after the read: {e}"),
        }
    }
}

impl<E: core::error::Error + 'static> core::error::Error for UpdateError<E> {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Read(e) | Self::Write(e) => Some(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Buf {
        bytes: [u8; 160],
        len: usize,
    }

    impl fmt::Write for Buf {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let end = self.len + s.len();
            self.bytes
                .get_mut(self.len..end)
                .ok_or(fmt::Error)?
                .copy_from_slice(s.as_bytes());
            self.len = end;
            Ok(())
        }
    }

    fn render(e: ParamsError) -> Buf {
        let mut buf = Buf {
            bytes: [0; 160],
            len: 0,
        };
        fmt::Write::write_fmt(&mut buf, format_args!("{e}")).unwrap();
        buf
    }

    fn message(rate_khz: u32, no_scdc: bool, no_scrambling_support: bool) -> Buf {
        message_with_source(rate_khz, no_scdc, no_scrambling_support, false)
    }

    fn message_with_source(
        rate_khz: u32,
        no_scdc: bool,
        no_scrambling_support: bool,
        source_cannot_scramble: bool,
    ) -> Buf {
        render(ParamsError::ScramblingUnsupported {
            rate_khz,
            no_scdc,
            no_scrambling_support,
            source_cannot_scramble,
        })
    }

    fn text(b: &Buf) -> &str {
        core::str::from_utf8(&b.bytes[..b.len]).unwrap()
    }

    #[test]
    fn display_names_every_missing_fact() {
        assert_eq!(
            text(&message(594_000, true, true)),
            "TMDS character rate 594000 kHz needs scrambling, but the sink has no SCDC \
             and does not support scrambling"
        );
        assert_eq!(
            text(&message(594_000, true, false)),
            "TMDS character rate 594000 kHz needs scrambling, but the sink has no SCDC"
        );
        assert_eq!(
            text(&message(340_001, false, true)),
            "TMDS character rate 340001 kHz needs scrambling, but the sink does not \
             support scrambling"
        );
    }

    #[test]
    fn display_names_a_source_without_a_scrambler() {
        assert_eq!(
            text(&message_with_source(594_000, false, false, true)),
            "TMDS character rate 594000 kHz needs scrambling, but the source has no scrambler"
        );
        assert_eq!(
            text(&message_with_source(594_000, true, false, true)),
            "TMDS character rate 594000 kHz needs scrambling, but the sink has no SCDC, and \
             the source has no scrambler"
        );
    }

    #[test]
    fn display_names_the_source_maximum() {
        let b = render(ParamsError::AboveSourceMaximum {
            rate_khz: 594_000,
            max_khz: 340_000,
        });
        assert_eq!(
            text(&b),
            "TMDS character rate 594000 kHz is above the source's maximum of 340000 kHz"
        );
    }

    #[test]
    fn display_without_a_missing_fact_still_reads() {
        // `choose` never returns this, and the variant is `#[non_exhaustive]`, so only
        // this crate can build it; the message still reads.
        assert_eq!(
            text(&message(594_000, false, false)),
            "TMDS character rate 594000 kHz needs scrambling, but the sink cannot take it"
        );
    }

    fn render_any(e: impl fmt::Display) -> Buf {
        let mut buf = Buf {
            bytes: [0; 160],
            len: 0,
        };
        fmt::Write::write_fmt(&mut buf, format_args!("{e}")).unwrap();
        buf
    }

    #[test]
    fn update_error_says_which_transfer_failed() {
        assert_eq!(UpdateError::Read(1).into_inner(), 1);
        assert_eq!(UpdateError::Write(2).into_inner(), 2);
        assert_eq!(
            UpdateError::Write(DdcErrorKind::Busy).kind(),
            DdcErrorKind::Busy
        );
        assert_eq!(
            text(&render_any(UpdateError::Read(
                ParamsError::AboveSourceMaximum {
                    rate_khz: 1,
                    max_khz: 0,
                }
            ))),
            "read failed, nothing written: TMDS character rate 1 kHz is above the source's \
             maximum of 0 kHz"
        );
        assert_eq!(
            text(&render_any(UpdateError::Write(
                ParamsError::AboveSourceMaximum {
                    rate_khz: 1,
                    max_khz: 0,
                }
            ))),
            "write failed after the read: TMDS character rate 1 kHz is above the source's \
             maximum of 0 kHz"
        );
    }

    #[test]
    fn update_error_sources_the_inner_error() {
        use core::error::Error;
        let inner = ParamsError::AboveSourceMaximum {
            rate_khz: 1,
            max_khz: 0,
        };
        for e in [UpdateError::Read(inner), UpdateError::Write(inner)] {
            let source = e.source().unwrap().downcast_ref::<ParamsError>();
            assert_eq!(source, Some(&inner));
        }
    }
}
