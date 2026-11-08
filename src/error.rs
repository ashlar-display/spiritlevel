//! The error `TmdsLinkParams::choose` returns: `ParamsError`.

use core::fmt;

/// Why [`TmdsLinkParams::choose`](crate::TmdsLinkParams::choose) refused the inputs.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParamsError {
    /// The rate is above 340 000 kHz, so the link must be scrambled, but the sink cannot
    /// take scrambling there. Every missing fact is set; at least one is.
    ScramblingUnsupported {
        /// The TMDS character rate asked for, in kHz.
        rate_khz: u32,
        /// The sink has no SCDC, so its `TMDS_Config` cannot be written.
        no_scdc: bool,
        /// The sink does not declare scrambling above 340 Mcsc.
        no_scrambling_above_340: bool,
    },
}

impl fmt::Display for ParamsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::ScramblingUnsupported {
                rate_khz,
                no_scdc,
                no_scrambling_above_340,
            } => {
                write!(
                    f,
                    "TMDS character rate {rate_khz} kHz needs scrambling, but the sink"
                )?;
                match (no_scdc, no_scrambling_above_340) {
                    (true, true) => {
                        f.write_str(" has no SCDC and does not declare scrambling above 340 Mcsc")
                    }
                    (true, false) => f.write_str(" has no SCDC"),
                    (false, true) => f.write_str(" does not declare scrambling above 340 Mcsc"),
                    (false, false) => f.write_str(" cannot take it"),
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

    fn message(rate_khz: u32, no_scdc: bool, no_scrambling_above_340: bool) -> Buf {
        render(ParamsError::ScramblingUnsupported {
            rate_khz,
            no_scdc,
            no_scrambling_above_340,
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
             and does not declare scrambling above 340 Mcsc"
        );
        assert_eq!(
            text(&message(594_000, true, false)),
            "TMDS character rate 594000 kHz needs scrambling, but the sink has no SCDC"
        );
        assert_eq!(
            text(&message(340_001, false, true)),
            "TMDS character rate 340001 kHz needs scrambling, but the sink does not \
             declare scrambling above 340 Mcsc"
        );
    }

    #[test]
    fn display_without_a_missing_fact_still_reads() {
        // `choose` never returns this; a caller can still build it.
        assert_eq!(
            text(&message(594_000, false, false)),
            "TMDS character rate 594000 kHz needs scrambling, but the sink cannot take it"
        );
    }
}
