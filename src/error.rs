//! The error `TmdsLinkParams::choose` returns: `ParamsError`.

use core::fmt;

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
        }
    }
}

impl core::error::Error for ParamsError {}

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
    fn display_without_a_missing_fact_still_reads() {
        // `choose` never returns this, and the variant is `#[non_exhaustive]`, so only
        // this crate can build it; the message still reads.
        assert_eq!(
            text(&message(594_000, false, false)),
            "TMDS character rate 594000 kHz needs scrambling, but the sink cannot take it"
        );
    }
}
